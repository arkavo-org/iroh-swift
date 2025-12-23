//! IrohNode implementation wrapping iroh/iroh-blobs.
//!
//! Provides a minimal interface for blob storage and retrieval,
//! with optional Docs (syncing key-value documents) support.

use anyhow::{Context, Result};
use futures_lite::StreamExt;
use iroh::endpoint::RelayMode;
use iroh::{Endpoint, RelayMap, RelayUrl, protocol::Router};
use iroh_blobs::api::downloader::DownloadProgressItem;
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
    /// Whether docs support is enabled.
    docs_enabled: bool,
    /// Gossip protocol for docs sync (only if docs_enabled).
    gossip: Option<Gossip>,
    /// Docs protocol (only if docs_enabled).
    docs: Option<Docs>,
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

        let (endpoint, store, router, gossip, docs) = runtime.block_on(async {
            // Create or load the persistent store
            let store = FsStore::load(&storage_path)
                .await
                .context("Failed to load blob store")?;

            // Build endpoint with relay configuration
            let mut builder = Endpoint::builder();
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

            // Wait for relay connection if enabled
            if relay_enabled {
                let _ = endpoint.online().await;
            }

            // Set up the blobs protocol handler
            let blobs = BlobsProtocol::new(&store, None);

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

            Ok::<_, anyhow::Error>((endpoint, store, router, gossip, docs))
        })?;

        Ok(Self {
            runtime,
            endpoint,
            store,
            router,
            docs_enabled,
            gossip,
            docs,
        })
    }

    /// Check if docs support is enabled.
    pub fn is_docs_enabled(&self) -> bool {
        self.docs_enabled
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
    pub fn shutdown(self) -> Result<()> {
        self.runtime.block_on(async {
            self.router
                .shutdown()
                .await
                .context("Failed to shutdown router")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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
