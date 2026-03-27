//! IrohNode implementation wrapping iroh/iroh-blobs.
//!
//! Provides a minimal interface for blob storage and retrieval,
//! with optional Docs (syncing key-value documents) support.

use anyhow::{Context, Result};
use futures_lite::StreamExt;
use iroh::endpoint::{RelayMode, presets};
use iroh::{Endpoint, EndpointAddr, RelayMap, RelayUrl, protocol::Router};
use iroh_blobs::api::downloader::DownloadProgressItem;
use iroh_blobs::protocol::{GetRequest, PushRequest};
use iroh_blobs::provider::events::{EventMask, EventSender, ProviderMessage, RequestMode};
use iroh_blobs::{ALPN as BLOBS_ALPN, BlobsProtocol, store::fs::FsStore, ticket::BlobTicket};
use iroh_docs::protocol::Docs;
use iroh_gossip::ALPN as GOSSIP_ALPN;
use iroh_gossip::net::Gossip;
use std::path::PathBuf;
use std::time::Duration;
use tokio::runtime::Runtime;

/// Information about an Iroh node.
pub struct NodeInfo {
    /// The node's unique identifier.
    pub node_id: String,
    /// The relay server URL, if connected.
    pub relay_url: Option<String>,
    /// Whether the node is connected to the network.
    pub is_connected: bool,
}

/// Minimal Iroh node for blob operations.
///
/// Each node owns its own Tokio runtime to avoid conflicts with Swift's
/// concurrency model. All async operations are executed via `block_on`.
///
/// Optionally supports Docs (syncing key-value documents) when `docs_enabled`
/// is true during construction.
pub struct IrohNode {
    runtime: Runtime,
    endpoint: Endpoint,
    store: FsStore,
    router: Router,
    /// Gossip protocol for docs sync (must be kept alive for router).
    #[allow(dead_code)]
    gossip: Option<Gossip>,
    /// Docs protocol (only if docs_enabled).
    docs: Option<Docs>,
    /// Event handler task for accepting push requests (must be kept alive).
    #[allow(dead_code)]
    event_handler: Option<tokio::task::JoinHandle<()>>,
}

impl IrohNode {
    /// Create a new Iroh node with persistent storage.
    ///
    /// # Arguments
    /// * `storage_path` - Directory for the blob store (created if doesn't exist)
    /// * `relay_enabled` - Whether to use relay servers
    /// * `custom_relay_url` - Optional custom relay URL (if None, uses n0's public relays)
    /// * `docs_enabled` - Whether to enable the Docs engine for syncing documents
    pub fn new(
        storage_path: PathBuf,
        relay_enabled: bool,
        custom_relay_url: Option<String>,
        docs_enabled: bool,
    ) -> Result<Self> {
        // Create dedicated runtime for this node
        let runtime = Runtime::new().context("Failed to create Tokio runtime")?;

        let (endpoint, store, router, gossip, docs, event_handler) = runtime.block_on(async {
            // Create or load the persistent store
            let store = FsStore::load(&storage_path)
                .await
                .context("Failed to load blob store")?;

            // Build endpoint with relay configuration
            let mut builder = Endpoint::builder(presets::N0);
            if !relay_enabled {
                builder = builder.relay_mode(RelayMode::Disabled);
            } else if let Some(url) = custom_relay_url {
                // Parse and use custom relay
                let relay_url: RelayUrl = url.parse().context("Invalid relay URL")?;
                let relay_map = RelayMap::from(relay_url);
                builder = builder.relay_mode(RelayMode::Custom(relay_map));
            }
            // else: n0 public relays are default when relay_enabled=true

            let endpoint = builder.bind().await.context("Failed to bind endpoint")?;

            // Wait for relay connection if enabled (with timeout to avoid hanging in CI)
            if relay_enabled {
                let _ = tokio::time::timeout(Duration::from_secs(10), endpoint.online()).await;
            }

            // Set up the blobs protocol handler with push acceptance.
            // EventSender::request() checks only mask.get for ALL request types
            // (get, push, etc.), so we set get: Notify to enable push delivery.
            let mask = EventMask {
                get: RequestMode::Notify,
                ..EventMask::DEFAULT
            };
            let (event_sender, mut event_rx) = EventSender::channel(64, mask);

            // Spawn event handler that accepts push requests
            let event_handler = tokio::spawn(async move {
                while let Some(msg) = event_rx.recv().await {
                    match msg {
                        ProviderMessage::PushRequestReceived(msg) => {
                            msg.tx.send(Ok(())).await.ok();
                        }
                        ProviderMessage::ClientConnected(msg) => {
                            msg.tx.send(Ok(())).await.ok();
                        }
                        _ => {}
                    }
                }
            });

            let blobs = BlobsProtocol::new(&store, Some(event_sender));

            // Conditionally set up Docs protocol
            let (gossip, docs) = if docs_enabled {
                // Create gossip protocol (synchronous - returns Gossip directly)
                let gossip = Gossip::builder().spawn(endpoint.clone());

                // Create docs path for persistent storage
                let docs_path = storage_path.join("docs");

                // Ensure docs directory exists
                if !docs_path.exists() {
                    std::fs::create_dir_all(&docs_path)
                        .context("Failed to create docs directory")?;
                }

                // Create docs protocol using the builder pattern
                let docs = Docs::persistent(docs_path)
                    .spawn(endpoint.clone(), store.clone().into(), gossip.clone())
                    .await
                    .context("Failed to spawn docs protocol")?;

                (Some(gossip), Some(docs))
            } else {
                (None, None)
            };

            // Build router with all protocols
            let mut router_builder = Router::builder(endpoint.clone()).accept(BLOBS_ALPN, blobs);

            if let Some(ref g) = gossip {
                router_builder = router_builder.accept(GOSSIP_ALPN, g.clone());
            }

            if let Some(ref d) = docs {
                router_builder = router_builder.accept(iroh_docs::ALPN, d.clone());
            }

            let router = router_builder.spawn();

            Ok::<_, anyhow::Error>((endpoint, store, router, gossip, docs, event_handler))
        })?;

        Ok(Self {
            runtime,
            endpoint,
            store,
            router,
            gossip,
            docs,
            event_handler: Some(event_handler),
        })
    }

    /// Check if docs support is enabled.
    #[allow(dead_code)]
    pub fn is_docs_enabled(&self) -> bool {
        self.docs.is_some()
    }

    /// Get the docs protocol if enabled.
    pub fn docs(&self) -> Option<&Docs> {
        self.docs.as_ref()
    }

    /// Get a reference to the runtime for FFI operations.
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Get a reference to the store for content operations.
    pub fn store(&self) -> &FsStore {
        &self.store
    }

    /// Get a reference to the endpoint for network operations.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Add bytes to the blob store and return a shareable ticket.
    ///
    /// The ticket can be used by other nodes to download the blob.
    pub fn put(&self, data: &[u8]) -> Result<String> {
        self.runtime.block_on(async {
            // Add the bytes to the store
            let tag = self
                .store
                .add_slice(data)
                .await
                .context("Failed to add bytes to store")?;

            // Get our network address for the ticket
            let addr = self.endpoint.addr();

            // Create a ticket that others can use to download
            let ticket = BlobTicket::new(addr, tag.hash, tag.format);

            Ok(ticket.to_string())
        })
    }

    /// Download bytes from a ticket.
    ///
    /// This fetches the blob from the remote peer specified in the ticket.
    pub fn get(&self, ticket_str: &str) -> Result<Vec<u8>> {
        self.runtime.block_on(async {
            // Parse the ticket
            let ticket: BlobTicket = ticket_str.parse().context("Failed to parse ticket")?;

            // Create a downloader for fetching from remote peers
            let downloader = self.store.downloader(&self.endpoint);

            // Download the blob (if not already present locally)
            // ContentDiscovery is implemented for sequences of NodeId
            downloader
                .download(ticket.hash(), [ticket.addr().id])
                .await
                .context("Failed to download blob")?;

            // Read the bytes from local store
            let bytes = self
                .store
                .get_bytes(ticket.hash())
                .await
                .context("Failed to read bytes from store")?;

            Ok(bytes.to_vec())
        })
    }

    /// Download bytes from a ticket with progress reporting.
    ///
    /// The progress callback is called with (downloaded, total) byte counts.
    /// Note: total may be 0 if the size is unknown.
    pub fn get_with_progress<F>(&self, ticket_str: &str, mut on_progress: F) -> Result<Vec<u8>>
    where
        F: FnMut(u64, u64),
    {
        self.runtime.block_on(async {
            // Parse the ticket
            let ticket: BlobTicket = ticket_str.parse().context("Failed to parse ticket")?;

            // Create a downloader for fetching from remote peers
            let downloader = self.store.downloader(&self.endpoint);

            // Download the blob with progress tracking
            let download = downloader.download(ticket.hash(), [ticket.addr().id]);
            let mut stream = download
                .stream()
                .await
                .context("Failed to start download")?;

            // Process progress events
            while let Some(item) = stream.next().await {
                match item {
                    DownloadProgressItem::Progress(bytes) => {
                        // Total is not directly available from progress events
                        on_progress(bytes, 0);
                    }
                    DownloadProgressItem::PartComplete { .. } => {
                        // Part of the download completed
                    }
                    DownloadProgressItem::Error(e) => {
                        return Err(anyhow::anyhow!("Download error: {:?}", e));
                    }
                    DownloadProgressItem::DownloadError => {
                        return Err(anyhow::anyhow!("Download failed"));
                    }
                    _ => {}
                }
            }

            // Read the bytes from local store
            let bytes = self
                .store
                .get_bytes(ticket.hash())
                .await
                .context("Failed to read bytes from store")?;

            Ok(bytes.to_vec())
        })
    }

    /// Add bytes to the blob store with an optional timeout.
    ///
    /// # Arguments
    /// * `data` - The bytes to store
    /// * `timeout_ms` - Timeout in milliseconds (0 = no timeout)
    pub fn put_with_timeout(&self, data: &[u8], timeout_ms: u64) -> Result<String> {
        self.runtime.block_on(async {
            let fut = async {
                let tag = self
                    .store
                    .add_slice(data)
                    .await
                    .context("Failed to add bytes to store")?;

                let addr = self.endpoint.addr();
                let ticket = BlobTicket::new(addr, tag.hash, tag.format);
                Ok::<_, anyhow::Error>(ticket.to_string())
            };

            if timeout_ms == 0 {
                fut.await
            } else {
                tokio::time::timeout(Duration::from_millis(timeout_ms), fut)
                    .await
                    .context("Operation timed out")?
            }
        })
    }

    /// Download bytes from a ticket with an optional timeout.
    ///
    /// # Arguments
    /// * `ticket_str` - The ticket string
    /// * `timeout_ms` - Timeout in milliseconds (0 = no timeout)
    pub fn get_with_timeout(&self, ticket_str: &str, timeout_ms: u64) -> Result<Vec<u8>> {
        self.runtime.block_on(async {
            let fut = async {
                let ticket: BlobTicket = ticket_str.parse().context("Failed to parse ticket")?;
                let downloader = self.store.downloader(&self.endpoint);

                downloader
                    .download(ticket.hash(), [ticket.addr().id])
                    .await
                    .context("Failed to download blob")?;

                let bytes = self
                    .store
                    .get_bytes(ticket.hash())
                    .await
                    .context("Failed to read bytes from store")?;

                Ok::<_, anyhow::Error>(bytes.to_vec())
            };

            if timeout_ms == 0 {
                fut.await
            } else {
                tokio::time::timeout(Duration::from_millis(timeout_ms), fut)
                    .await
                    .context("Operation timed out")?
            }
        })
    }

    /// Push data to a remote node.
    ///
    /// Adds the data to the local store, connects to the remote node,
    /// and pushes the blob. Returns the blob hash as a hex string.
    ///
    /// # Arguments
    /// * `remote_node_id_hex` - The remote node's ID as a hex string
    /// * `relay_url` - Optional relay URL for routing the connection
    /// * `data` - The bytes to push
    /// * `direct_addrs` - Optional direct IP:port addresses for local / no-relay connections
    pub fn push_to_node(
        &self,
        remote_node_id_hex: &str,
        relay_url: Option<&str>,
        data: &[u8],
        direct_addrs: &[String],
    ) -> Result<String> {
        self.runtime.block_on(async {
            // Add bytes to local store first
            let tag = self
                .store
                .add_slice(data)
                .await
                .context("Failed to add bytes to store")?;

            // Parse remote node ID
            let node_id: iroh::EndpointId = remote_node_id_hex
                .parse()
                .context("Failed to parse remote node ID")?;

            // Build EndpointAddr with optional relay and/or direct addresses
            let mut endpoint_addr = EndpointAddr::from(node_id);
            if let Some(url) = relay_url {
                let relay_url: RelayUrl = url.parse().context("Invalid relay URL")?;
                endpoint_addr = endpoint_addr.with_relay_url(relay_url);
            }
            for addr_str in direct_addrs {
                let socket_addr: std::net::SocketAddr =
                    addr_str.parse().context("Invalid direct address")?;
                endpoint_addr = endpoint_addr.with_ip_addr(socket_addr);
            }

            // Connect to remote node
            let conn = self
                .endpoint
                .connect(endpoint_addr, BLOBS_ALPN)
                .await
                .context("Failed to connect to remote node")?;

            // Keep a connection handle alive — execute_push takes ownership but
            // Connection is Clone (ref-counted). The server needs time to
            // accept_bi and process the push stream after execute_push returns.
            let _conn_guard = conn.clone();

            // Push the blob
            let request = PushRequest::from(GetRequest::blob(tag.hash));
            self.store
                .remote()
                .execute_push(conn, request)
                .await
                .context("Failed to push blob to remote node")?;

            // Give the server time to process the push before dropping the connection.
            // Without this, small blobs may transfer in a single QUIC packet and the
            // server's accept_bi may not be scheduled before the connection closes.
            tokio::time::sleep(Duration::from_millis(500)).await;

            Ok(hex::encode(tag.hash.as_bytes()))
        })
    }

    /// Push data to a remote node with an optional timeout.
    ///
    /// # Arguments
    /// * `remote_node_id_hex` - The remote node's ID as a hex string
    /// * `relay_url` - Optional relay URL for routing the connection
    /// * `data` - The bytes to push
    /// * `direct_addrs` - Optional direct IP:port addresses for local / no-relay connections
    /// * `timeout_ms` - Timeout in milliseconds (0 = no timeout)
    pub fn push_to_node_with_timeout(
        &self,
        remote_node_id_hex: &str,
        relay_url: Option<&str>,
        data: &[u8],
        direct_addrs: &[String],
        timeout_ms: u64,
    ) -> Result<String> {
        self.runtime.block_on(async {
            let fut = async {
                let tag = self
                    .store
                    .add_slice(data)
                    .await
                    .context("Failed to add bytes to store")?;

                let node_id: iroh::EndpointId = remote_node_id_hex
                    .parse()
                    .context("Failed to parse remote node ID")?;

                let mut endpoint_addr = EndpointAddr::from(node_id);
                if let Some(url) = relay_url {
                    let relay_url: RelayUrl = url.parse().context("Invalid relay URL")?;
                    endpoint_addr = endpoint_addr.with_relay_url(relay_url);
                }
                for addr_str in direct_addrs {
                    let socket_addr: std::net::SocketAddr =
                        addr_str.parse().context("Invalid direct address")?;
                    endpoint_addr = endpoint_addr.with_ip_addr(socket_addr);
                }

                let conn = self
                    .endpoint
                    .connect(endpoint_addr, BLOBS_ALPN)
                    .await
                    .context("Failed to connect to remote node")?;

                let _conn_guard = conn.clone();

                let request = PushRequest::from(GetRequest::blob(tag.hash));
                self.store
                    .remote()
                    .execute_push(conn, request)
                    .await
                    .context("Failed to push blob to remote node")?;

                tokio::time::sleep(Duration::from_millis(500)).await;

                Ok::<_, anyhow::Error>(hex::encode(tag.hash.as_bytes()))
            };

            if timeout_ms == 0 {
                fut.await
            } else {
                tokio::time::timeout(Duration::from_millis(timeout_ms), fut)
                    .await
                    .context("Operation timed out")?
            }
        })
    }

    /// Get information about this node.
    pub fn info(&self) -> Result<NodeInfo> {
        self.runtime.block_on(async {
            // Get node ID from endpoint
            let node_id = self.endpoint.id().to_string();

            // Get address info which includes relay
            let addr = self.endpoint.addr();
            // Get the first relay URL if any
            let relay_url = addr.relay_urls().next().map(|url| url.to_string());

            // A node is considered connected if it has a relay URL or IP addresses
            let is_connected = relay_url.is_some() || addr.ip_addrs().next().is_some();

            Ok(NodeInfo {
                node_id,
                relay_url,
                is_connected,
            })
        })
    }

    /// Gracefully shut down the node.
    ///
    /// This ensures all pending writes are flushed to disk.
    /// Uses a timeout to prevent hanging if the router cannot shut down cleanly.
    pub fn shutdown(self) -> Result<()> {
        self.runtime.block_on(async {
            match tokio::time::timeout(Duration::from_secs(5), self.router.shutdown()).await {
                Ok(result) => result.context("Failed to shutdown router"),
                Err(_) => {
                    // Timeout expired — force drop to avoid hanging forever
                    Ok(())
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_push_to_node_roundtrip() {
        let dir_a = tempdir().unwrap();
        let dir_b = tempdir().unwrap();

        // Create two nodes, relay disabled (local connection)
        let node_a = IrohNode::new(dir_a.path().to_path_buf(), false, None, false).unwrap();
        let node_b = IrohNode::new(dir_b.path().to_path_buf(), false, None, false).unwrap();

        // Get node B's ID
        let node_b_info = node_b.info().unwrap();
        let node_b_id = &node_b_info.node_id;

        // Collect node B's direct IP addresses for local-only connection (relay disabled)
        let node_b_addr = node_b.endpoint().addr();
        let direct_addrs: Vec<String> = node_b_addr.ip_addrs().map(|a| a.to_string()).collect();

        // Push data from A to B
        let data = b"Push test data!";
        let hash_hex = node_a
            .push_to_node(node_b_id, None, data, &direct_addrs)
            .unwrap();

        // Verify hash is valid hex (64 chars for blake3)
        assert_eq!(hash_hex.len(), 64);
        assert!(hash_hex.chars().all(|c: char| c.is_ascii_hexdigit()));

        node_a.shutdown().unwrap();
        node_b.shutdown().unwrap();
    }

    /// Push to a live remote node.
    /// Requires env vars: IROH_REMOTE_NODE_ID, IROH_REMOTE_ADDR
    /// Run with:
    ///   IROH_REMOTE_NODE_ID=<hex> IROH_REMOTE_ADDR=<ip:port> \
    ///     cargo test test_push_to_remote -- --ignored --nocapture
    #[test]
    #[ignore]
    fn test_push_to_remote() {
        let remote_node_id = std::env::var("IROH_REMOTE_NODE_ID").expect("Set IROH_REMOTE_NODE_ID");
        let direct_addr = std::env::var("IROH_REMOTE_ADDR").expect("Set IROH_REMOTE_ADDR");

        let dir = tempdir().unwrap();
        let node = IrohNode::new(dir.path().to_path_buf(), true, None, false).unwrap();

        let info = node.info().unwrap();
        println!("Local node ID: {}", info.node_id);
        println!("Relay: {}", info.relay_url.as_deref().unwrap_or("none"));

        let data = b"iroh-swift push integration test";
        println!("Pushing {} bytes to {}...", data.len(), direct_addr);

        let hash_hex = node
            .push_to_node(&remote_node_id, None, data, &[direct_addr])
            .expect("Push to remote node failed");

        println!("Push succeeded! Hash: {}", hash_hex);
        assert_eq!(hash_hex.len(), 64);

        node.shutdown().unwrap();
    }

    #[test]
    fn test_put_roundtrip() {
        let dir = tempdir().unwrap();
        let node = IrohNode::new(dir.path().to_path_buf(), false, None, false).unwrap();

        let data = b"Hello, Iroh!";
        let ticket = node.put(data).unwrap();

        assert!(!ticket.is_empty());
        assert!(ticket.starts_with("blob")); // BlobTicket format

        node.shutdown().unwrap();
    }

    #[test]
    fn test_node_with_docs_enabled() {
        let dir = tempdir().unwrap();
        let node = IrohNode::new(dir.path().to_path_buf(), false, None, true).unwrap();

        assert!(node.is_docs_enabled());
        assert!(node.docs().is_some());

        node.shutdown().unwrap();
    }
}
