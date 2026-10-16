//! IrohNode implementation wrapping iroh/iroh-blobs.
//!
//! Provides a minimal interface for blob storage and retrieval,
//! with optional Docs (syncing key-value documents) support.

use anyhow::{Context, Result};
use bao_tree::io::BaoContentItem;
use futures_lite::StreamExt;
use iroh::endpoint::{Connection, ReadError, RelayMode, VarInt, presets};
use iroh::{Endpoint, EndpointAddr, RelayMap, RelayUrl, protocol::Router};
use iroh_blobs::api::downloader::DownloadProgressItem;
use iroh_blobs::get::fsm::{
    AtBlobHeaderNextError, BlobContentNext, ConnectedNext, DecodeError, EndBlobNext,
};
use iroh_blobs::protocol::{ERR_INTERNAL, ERR_PERMISSION, GetRequest, PushRequest};
use iroh_blobs::provider::events::{EventMask, EventSender, ProviderMessage, RequestMode};
use iroh_blobs::{ALPN as BLOBS_ALPN, BlobsProtocol, Hash, store::fs::FsStore, ticket::BlobTicket};
use iroh_docs::protocol::Docs;
use iroh_gossip::ALPN as GOSSIP_ALPN;
use iroh_gossip::net::Gossip;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

/// Information about an Iroh node.
pub struct NodeInfo {
    /// The node's unique identifier.
    pub node_id: String,
    /// The relay server URL, if connected.
    pub relay_url: Option<String>,
    /// Whether the node is connected to the network.
    pub is_connected: bool,
    /// Direct `ip:port` socket addresses this node is reachable on.
    pub direct_addrs: Vec<String>,
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
    /// Parent of every spawned, cancellable operation (fetch).
    /// Cancelled first on shutdown so in-flight operations report
    /// [`OperationError::NodeShutDown`] instead of an opaque connection error.
    operations: CancellationToken,
}

impl IrohNode {
    /// Create a new Iroh node with persistent storage.
    ///
    /// # Arguments
    /// * `storage_path` - Directory for the blob store (created if doesn't exist)
    /// * `relay_enabled` - Whether to use relay servers
    /// * `custom_relay_url` - Optional custom relay URL (if None, uses n0's public relays)
    /// * `docs_enabled` - Whether to enable the Docs engine for syncing documents
    /// * `serves_blobs` - Whether to accept the iroh-blobs ALPN. When false the node
    ///   is client-only: it can still fetch and push, but peers cannot get blobs
    ///   from it (or push to it).
    pub fn new(
        storage_path: PathBuf,
        relay_enabled: bool,
        custom_relay_url: Option<String>,
        docs_enabled: bool,
        serves_blobs: bool,
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

            // Set up the blobs protocol handler with push acceptance, unless the
            // node is client-only. Fetching (`store.remote()`, the downloader) and
            // pushing only need the store and the endpoint, not the handler.
            let (blobs, event_handler) = if serves_blobs {
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
                (Some(blobs), Some(event_handler))
            } else {
                (None, None)
            };

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

            // Build router with all protocols. A client-only node registers no
            // blobs ALPN, so incoming blobs connections fail the TLS/ALPN handshake.
            let mut router_builder = Router::builder(endpoint.clone());

            if let Some(blobs) = blobs {
                router_builder = router_builder.accept(BLOBS_ALPN, blobs);
            }

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
            event_handler,
            operations: CancellationToken::new(),
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

    /// Whether the node has started shutting down.
    pub fn is_shutting_down(&self) -> bool {
        self.operations.is_cancelled()
    }

    /// Cancelled when the node starts shutting down; pass it to
    /// [`run_cancellable`] so in-flight operations stop with
    /// [`OperationError::NodeShutDown`].
    pub fn shutdown_token(&self) -> CancellationToken {
        self.operations.clone()
    }

    /// This endpoint's id: the Ed25519 public key as 64 lowercase hex characters.
    ///
    /// Same string as [`NodeInfo::node_id`] and as `EndpointId`'s `Display`,
    /// which is what a peer (e.g. tdf-iroh-s3) puts in a signed message.
    pub fn endpoint_id(&self) -> String {
        self.endpoint.id().to_string()
    }

    /// Sign `message` with this endpoint's Ed25519 secret key.
    ///
    /// Returns the 64-byte signature. It verifies (strictly) against the public
    /// key in [`Self::endpoint_id`]. The key is generated when the node is
    /// created and is not persisted, so it changes with every node.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.endpoint.secret_key().sign(message).to_bytes()
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

            let direct_addrs = addr.ip_addrs().map(|a| a.to_string()).collect();

            Ok(NodeInfo {
                node_id,
                relay_url,
                is_connected,
                direct_addrs,
            })
        })
    }

    /// Gracefully shut down the node.
    ///
    /// This ensures all pending writes are flushed to disk.
    /// Uses a timeout to prevent hanging if the router cannot shut down cleanly.
    /// The Tokio runtime is also shut down with a timeout to avoid blocking on
    /// background tasks (e.g., event handlers) that may never complete.
    pub fn shutdown(self) -> Result<()> {
        // Destructure to control drop order: drop all node resources first,
        // then shut down the runtime last. This prevents Drop impls on
        // endpoint/store/etc from trying to use an already-shut-down runtime.
        let IrohNode {
            runtime,
            router,
            endpoint,
            store,
            gossip,
            docs,
            event_handler,
            operations,
        } = self;

        // Stop in-flight fetches first: each reports NodeShutDown and
        // removes its partial file before the endpoint goes away.
        operations.cancel();

        let result = runtime.block_on(async {
            match tokio::time::timeout(Duration::from_secs(5), router.shutdown()).await {
                Ok(result) => result.context("Failed to shutdown router"),
                Err(_) => Ok(()),
            }
        });

        // Drop all node resources before shutting down the runtime.
        drop(event_handler);
        drop(docs);
        drop(gossip);
        drop(store);
        drop(endpoint);

        // Explicitly shut down the runtime with a timeout so that
        // background tasks (event handler, etc.) don't block process exit.
        runtime.shutdown_timeout(Duration::from_secs(2));

        result
    }
}

/// Build the address of a remote node from its hex id, an optional relay URL
/// and optional direct `ip:port` addresses.
///
/// With neither a relay URL nor direct addresses, iroh resolves the node
/// through the endpoint's address lookup (n0 DNS/pkarr with the default preset).
pub fn remote_endpoint_addr(
    remote_node_id_hex: &str,
    relay_url: Option<&str>,
    direct_addrs: &[String],
) -> Result<EndpointAddr> {
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
    Ok(endpoint_addr)
}

/// Why a spawned operation (fetch) did not complete.
#[derive(Debug)]
pub enum OperationError {
    /// The remote does not have the requested blob (or not all of it).
    NotFound,
    /// The remote announced a blob size larger than the caller's limit.
    SizeLimitExceeded(u64),
    /// The caller cancelled the operation.
    Cancelled,
    /// The node shut down while the operation was in flight.
    NodeShutDown,
    /// Anything else: bad arguments, connection, protocol, verification or I/O.
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for OperationError {
    fn from(e: anyhow::Error) -> Self {
        OperationError::Failed(e)
    }
}

impl std::fmt::Display for OperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OperationError::NotFound => write!(f, "blob not found on the remote node"),
            OperationError::SizeLimitExceeded(size) => {
                write!(f, "blob size {size} exceeds the size limit")
            }
            OperationError::Cancelled => write!(f, "operation cancelled"),
            OperationError::NodeShutDown => write!(f, "node shut down"),
            OperationError::Failed(e) => write!(f, "{e:#}"),
        }
    }
}

/// Run `fut` until it completes, `cancel` is cancelled, or `shutdown` (the
/// node's [`IrohNode::shutdown_token`]) is cancelled.
///
/// Pass the same `cancel` token into `fut` if it should also check for
/// cancellation between steps that never pend (as [`fetch_to_path`] does).
pub async fn run_cancellable<T>(
    fut: impl std::future::Future<Output = Result<T, OperationError>>,
    cancel: &CancellationToken,
    shutdown: &CancellationToken,
) -> Result<T, OperationError> {
    let result = tokio::select! {
        biased;
        _ = shutdown.cancelled() => Err(OperationError::NodeShutDown),
        _ = cancel.cancelled() => Err(OperationError::Cancelled),
        r = fut => r,
    };
    // The future (and any partial file it owned) is dropped by now.
    match result {
        Err(OperationError::Cancelled) if shutdown.is_cancelled() => {
            Err(OperationError::NodeShutDown)
        }
        other => other,
    }
}

/// How often (in downloaded bytes) a fetch reports progress, besides the
/// first report (0 bytes, once the size is known) and the final one.
pub const FETCH_PROGRESS_STEP: u64 = 256 * 1024;

/// Removes a temporary file when dropped, unless it was committed.
struct TempFileGuard {
    path: Option<PathBuf>,
}

impl TempFileGuard {
    fn commit(mut self) {
        self.path = None;
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Sibling temp path for `destination`, so the final rename stays on one volume.
fn temp_path_for(destination: &Path) -> Result<PathBuf> {
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .context("Destination has no parent directory")?;
    let name = destination
        .file_name()
        .context("Destination has no file name")?
        .to_string_lossy();
    let nonce: u64 = rand::random();
    Ok(parent.join(format!(".{name}.{nonce:016x}.iroh-partial")))
}

/// Map a failure to read the size header (the first bytes of a response).
fn header_error(e: AtBlobHeaderNextError) -> OperationError {
    match e {
        // The provider finished the stream without sending anything.
        AtBlobHeaderNextError::NotFound { .. } => OperationError::NotFound,
        AtBlobHeaderNextError::Read { source, .. } => {
            let reset_code = source
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<ReadError>())
                .and_then(|re| match re {
                    ReadError::Reset(code) => Some(*code),
                    _ => None,
                });
            match reset_code {
                // An iroh-blobs provider resets the stream with ERR_INTERNAL
                // before the size header when exporting the blob fails, which
                // for a hash the store does not have is "no entry found".
                Some(code) if code == ERR_INTERNAL => OperationError::NotFound,
                Some(code) if code == ERR_PERMISSION => OperationError::Failed(anyhow::anyhow!(
                    "Remote node refused the request (permission denied)"
                )),
                _ => OperationError::Failed(
                    anyhow::Error::new(source).context("Failed to read blob size from remote node"),
                ),
            }
        }
    }
}

/// Map a failure while streaming verified content.
fn decode_error(e: DecodeError) -> OperationError {
    match e {
        // The provider stopped sending because it lacks (part of) the blob.
        DecodeError::ChunkNotFound { .. }
        | DecodeError::ParentNotFound { .. }
        | DecodeError::LeafNotFound { .. } => OperationError::NotFound,
        DecodeError::ParentHashMismatch { .. } | DecodeError::LeafHashMismatch { .. } => {
            OperationError::Failed(anyhow::anyhow!(
                "Blob verification failed: remote sent data that does not match the hash ({e})"
            ))
        }
        other => OperationError::Failed(
            anyhow::Error::new(other).context("Failed while receiving blob from remote node"),
        ),
    }
}

/// Fetch the raw blob `hash` from one remote node straight into `destination`.
///
/// Uses the iroh-blobs get state machine, so every byte written is verified
/// against `hash` (BLAKE3 / bao) as it arrives, and nothing is added to the
/// local store. The size the remote announces is checked against `max_size`
/// before any content is read; the cumulative byte count is checked again while
/// streaming. Content goes to a hidden sibling temp file that is renamed to
/// `destination` only after the whole blob has been verified and synced; on any
/// failure, cancellation or drop the temp file is removed and `destination` is
/// left untouched.
///
/// `on_progress(downloaded, total)` is called with `(0, total)` once the size
/// is accepted, then at least every [`FETCH_PROGRESS_STEP`] bytes, and finally
/// with `(total, total)`. Values are cumulative and never decrease.
///
/// Returns the blob size in bytes.
pub async fn fetch_to_path(
    endpoint: &Endpoint,
    hash: Hash,
    remote: EndpointAddr,
    destination: &Path,
    max_size: u64,
    cancel: &CancellationToken,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<u64, OperationError> {
    let temp_path = temp_path_for(destination)?;

    let conn = endpoint
        .connect(remote, BLOBS_ALPN)
        .await
        .context("Failed to connect to remote node")?;

    let result = fetch_on_connection(
        &conn,
        hash,
        destination,
        &temp_path,
        max_size,
        cancel,
        &mut on_progress,
    )
    .await;

    // Tell the provider to stop sending (no-op if it already finished).
    conn.close(VarInt::from_u32(0), b"done");
    result
}

async fn fetch_on_connection(
    conn: &Connection,
    hash: Hash,
    destination: &Path,
    temp_path: &Path,
    max_size: u64,
    cancel: &CancellationToken,
    on_progress: &mut impl FnMut(u64, u64),
) -> Result<u64, OperationError> {
    use iroh_blobs::get::fsm;

    let connected = fsm::start(conn.clone(), GetRequest::blob(hash), Default::default())
        .next()
        .await
        .context("Failed to open stream to remote node")?;
    let ConnectedNext::StartRoot(root) = connected
        .next()
        .await
        .context("Failed to send request to remote node")?
    else {
        return Err(anyhow::anyhow!("Unexpected response shape for a single-blob request").into());
    };

    // The size header comes before any content: enforce the limit here.
    let (mut content, size) = root.next().next().await.map_err(header_error)?;
    if size > max_size {
        return Err(OperationError::SizeLimitExceeded(size));
    }

    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp_path)
        .await
        .with_context(|| format!("Failed to create temporary file {}", temp_path.display()))?;
    let guard = TempFileGuard {
        path: Some(temp_path.to_path_buf()),
    };
    let mut writer = tokio::io::BufWriter::with_capacity(1024 * 1024, file);

    on_progress(0, size);
    let mut downloaded: u64 = 0;
    let mut reported: u64 = 0;
    let mut position: u64 = 0;

    let end = loop {
        match content.next().await {
            BlobContentNext::More((next, item)) => {
                content = next;
                match item.map_err(decode_error)? {
                    BaoContentItem::Parent(_) => {}
                    BaoContentItem::Leaf(leaf) => {
                        let len = leaf.data.len() as u64;
                        let leaf_end = leaf.offset.saturating_add(len);
                        if leaf_end > size {
                            return Err(anyhow::anyhow!(
                                "Remote sent data past the announced size ({leaf_end} > {size})"
                            )
                            .into());
                        }
                        downloaded += len;
                        if downloaded > max_size {
                            return Err(OperationError::SizeLimitExceeded(downloaded));
                        }
                        if leaf.offset != position {
                            writer
                                .seek(std::io::SeekFrom::Start(leaf.offset))
                                .await
                                .context("Failed to seek temporary file")?;
                        }
                        writer
                            .write_all(&leaf.data)
                            .await
                            .context("Failed to write temporary file")?;
                        position = leaf_end;

                        if downloaded == size || downloaded - reported >= FETCH_PROGRESS_STEP {
                            on_progress(downloaded, size);
                            reported = downloaded;
                        }
                        // Honour cancellation between leaves too, so a transfer
                        // whose reads never pend still stops promptly.
                        if cancel.is_cancelled() {
                            return Err(OperationError::Cancelled);
                        }
                    }
                }
            }
            BlobContentNext::Done(end) => break end,
        }
    };

    if downloaded != size {
        return Err(anyhow::anyhow!("Remote finished after {downloaded} of {size} bytes").into());
    }

    // All content is verified. Reading the end of the response only checks
    // for trailing data, so a failure there does not invalidate the blob.
    if let EndBlobNext::Closing(closing) = end.next() {
        let _ = closing.next().await;
    }

    writer
        .flush()
        .await
        .context("Failed to flush temporary file")?;
    let file = writer.into_inner();
    file.sync_all()
        .await
        .context("Failed to sync temporary file")?;
    drop(file);

    if reported != size {
        on_progress(size, size);
    }

    // No await between the rename and returning: once the file is in place,
    // the operation reports success.
    std::fs::rename(temp_path, destination)
        .with_context(|| format!("Failed to move blob to {}", destination.display()))?;
    guard.commit();
    Ok(size)
}

/// BLAKE3 hash of `data` as 64 lowercase hex characters.
pub fn blake3_hex(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    fn local_node(dir: &tempfile::TempDir, serves_blobs: bool) -> IrohNode {
        IrohNode::new(dir.path().to_path_buf(), false, None, false, serves_blobs).unwrap()
    }

    fn direct_addrs(node: &IrohNode) -> Vec<String> {
        node.info().unwrap().direct_addrs
    }

    /// Deterministic, non-repeating-looking test content.
    fn test_bytes(len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| (i % 251) as u8 ^ (i / 251) as u8)
            .collect()
    }

    /// Fetch on `client` from `server` and collect progress reports.
    fn fetch(
        client: &IrohNode,
        server: &IrohNode,
        hash: Hash,
        destination: &Path,
        max_size: u64,
    ) -> (Result<u64, OperationError>, Vec<(u64, u64)>) {
        let addr =
            remote_endpoint_addr(&server.endpoint_id(), None, &direct_addrs(server)).unwrap();
        let token = CancellationToken::new();
        let mut progress = Vec::new();
        let result = client.runtime().block_on(fetch_to_path(
            client.endpoint(),
            hash,
            addr,
            destination,
            max_size,
            &token,
            |d, t| progress.push((d, t)),
        ));
        (result, progress)
    }

    fn leftover_files(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn test_client_only_node_reports_direct_addrs() {
        let dir = tempdir().unwrap();
        let node = local_node(&dir, false);

        let info = node.info().unwrap();
        assert!(!info.direct_addrs.is_empty());
        for addr in &info.direct_addrs {
            addr.parse::<std::net::SocketAddr>()
                .unwrap_or_else(|e| panic!("{addr}: {e}"));
        }

        node.shutdown().unwrap();
    }

    #[test]
    fn test_endpoint_id_is_lowercase_hex_and_matches_info() {
        let dir = tempdir().unwrap();
        let node = local_node(&dir, true);

        let id = node.endpoint_id();
        assert_eq!(id.len(), 64);
        assert!(id.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
        assert_eq!(id, node.info().unwrap().node_id);
        assert_eq!(id, hex::encode(node.endpoint().id().as_bytes()));

        node.shutdown().unwrap();
    }

    /// The publish-session message tdf-iroh-s3 verifies with `verify_strict`.
    #[test]
    fn test_sign_verifies_strict_against_endpoint_id() {
        let dir = tempdir().unwrap();
        let node = local_node(&dir, false);

        let endpoint_id = node.endpoint_id();
        let message = format!(
            "tdf-iroh-s3 publish-session v1\n{endpoint_id}\n{}\n{}",
            blake3_hex(b"bearer-token"),
            1_760_000_000
        );
        let signature = node.sign(message.as_bytes());

        // Exactly what the server does: parse the id, verify (verify_strict inside).
        let public: iroh::EndpointId = endpoint_id.parse().unwrap();
        public
            .verify(message.as_bytes(), &iroh::Signature::from_bytes(&signature))
            .expect("iroh verify (verify_strict)");

        // And with ed25519-dalek directly.
        let key_bytes: [u8; 32] = hex::decode(&endpoint_id).unwrap().try_into().unwrap();
        let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&key_bytes).unwrap();
        let dalek_sig = ed25519_dalek::Signature::from_bytes(&signature);
        verifying_key
            .verify_strict(message.as_bytes(), &dalek_sig)
            .expect("ed25519-dalek verify_strict");

        // A different message must not verify.
        assert!(
            verifying_key
                .verify_strict(b"something else", &dalek_sig)
                .is_err()
        );

        node.shutdown().unwrap();
    }

    /// Official BLAKE3 test vectors (BLAKE3-team/BLAKE3 test_vectors.json:
    /// input is bytes `i % 251`), plus "abc"; all cross-checked with b3sum.
    #[test]
    fn test_blake3_hex_vectors() {
        let pattern = |len: usize| (0..len).map(|i| (i % 251) as u8).collect::<Vec<u8>>();
        assert_eq!(
            blake3_hex(b""),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        assert_eq!(
            blake3_hex(&pattern(1)),
            "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213"
        );
        assert_eq!(
            blake3_hex(&pattern(1024)),
            "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7"
        );
        assert_eq!(
            blake3_hex(&pattern(1025)),
            "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444"
        );
        assert_eq!(
            blake3_hex(b"abc"),
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
        );
    }

    #[test]
    fn test_fetch_to_path_from_server_to_client_only_node() {
        let (dir_a, dir_b, out) = (tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap());
        let server = local_node(&dir_a, true);
        let client = local_node(&dir_b, false);

        let data = test_bytes(3 * 1024 * 1024 + 123);
        let hash = server
            .runtime()
            .block_on(async { server.store().add_slice(&data).await })
            .unwrap()
            .hash;
        assert_eq!(hash.to_hex().to_string(), blake3_hex(&data));

        let dest = out.path().join("blob.bin");
        let (result, progress) = fetch(&client, &server, hash, &dest, data.len() as u64);
        assert_eq!(result.unwrap(), data.len() as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(leftover_files(out.path()), vec!["blob.bin".to_string()]);

        // Cumulative, monotonic, (0, total) first and (total, total) last.
        let total = data.len() as u64;
        assert!(progress.len() > 2, "expected several reports: {progress:?}");
        assert_eq!(progress.first(), Some(&(0, total)));
        assert_eq!(progress.last(), Some(&(total, total)));
        assert!(progress.windows(2).all(|w| w[0].0 <= w[1].0));
        assert!(progress.iter().all(|&(_, t)| t == total));

        // Nothing landed in the client's store.
        let has = client.runtime().block_on(client.store().has(hash)).unwrap();
        assert!(!has);

        client.shutdown().unwrap();
        server.shutdown().unwrap();
    }

    #[test]
    fn test_fetch_rejects_blob_over_size_limit_before_writing() {
        let (dir_a, dir_b, out) = (tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap());
        let server = local_node(&dir_a, true);
        let client = local_node(&dir_b, false);

        let data = test_bytes(100_000);
        let hash = server
            .runtime()
            .block_on(async { server.store().add_slice(&data).await })
            .unwrap()
            .hash;

        let dest = out.path().join("blob.bin");
        let (result, progress) = fetch(&client, &server, hash, &dest, data.len() as u64 - 1);
        match result {
            Err(OperationError::SizeLimitExceeded(size)) => assert_eq!(size, data.len() as u64),
            other => panic!("expected SizeLimitExceeded, got {other:?}"),
        }
        assert!(
            progress.is_empty(),
            "no progress before the size is accepted"
        );
        assert!(leftover_files(out.path()).is_empty());

        client.shutdown().unwrap();
        server.shutdown().unwrap();
    }

    #[test]
    fn test_fetch_unknown_hash_is_not_found() {
        let (dir_a, dir_b, out) = (tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap());
        let server = local_node(&dir_a, true);
        let client = local_node(&dir_b, false);

        let unknown = Hash::new(b"never stored anywhere");
        let dest = out.path().join("blob.bin");
        let (result, _) = fetch(&client, &server, unknown, &dest, u64::MAX);
        assert!(
            matches!(result, Err(OperationError::NotFound)),
            "expected NotFound, got {result:?}"
        );
        assert!(leftover_files(out.path()).is_empty());

        client.shutdown().unwrap();
        server.shutdown().unwrap();
    }

    #[test]
    fn test_fetch_from_client_only_node_fails() {
        let (dir_a, dir_b, out) = (tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap());
        let fetcher = local_node(&dir_a, true);
        let client_only = local_node(&dir_b, false);

        let data = b"not served".to_vec();
        let hash = client_only
            .runtime()
            .block_on(async { client_only.store().add_slice(&data).await })
            .unwrap()
            .hash;

        let dest = out.path().join("blob.bin");
        let (result, _) = fetch(&fetcher, &client_only, hash, &dest, u64::MAX);
        assert!(
            matches!(result, Err(OperationError::Failed(_))),
            "expected a connection failure, got {result:?}"
        );
        assert!(leftover_files(out.path()).is_empty());

        fetcher.shutdown().unwrap();
        client_only.shutdown().unwrap();
    }

    /// Cancelling mid-transfer stops the fetch and removes the partial file.
    #[test]
    fn test_fetch_cancel_mid_transfer_removes_partial_file() {
        let (dir_a, dir_b, out) = (tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap());
        let server = local_node(&dir_a, true);
        let client = local_node(&dir_b, false);

        let data = test_bytes(8 * 1024 * 1024);
        let hash = server
            .runtime()
            .block_on(async { server.store().add_slice(&data).await })
            .unwrap()
            .hash;

        let addr =
            remote_endpoint_addr(&server.endpoint_id(), None, &direct_addrs(&server)).unwrap();
        let dest = out.path().join("blob.bin");
        let token = CancellationToken::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let result = client.runtime().block_on(run_cancellable(
            fetch_to_path(
                client.endpoint(),
                hash,
                addr,
                &dest,
                u64::MAX,
                &token,
                |d, t| {
                    seen.lock().unwrap().push((d, t));
                    if d > 0 {
                        token.cancel();
                    }
                },
            ),
            &token,
            &client.shutdown_token(),
        ));
        assert!(
            matches!(result, Err(OperationError::Cancelled)),
            "expected Cancelled, got {result:?}"
        );
        let seen = seen.lock().unwrap();
        assert!(seen.last().unwrap().0 < data.len() as u64, "{seen:?}");
        assert!(leftover_files(out.path()).is_empty());

        client.shutdown().unwrap();
        server.shutdown().unwrap();
    }

    /// Cancelling while still connecting (nothing listens) returns promptly.
    #[test]
    fn test_fetch_cancel_while_connecting() {
        let (dir_a, dir_b, out) = (tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap());
        let absent = local_node(&dir_a, true);
        let absent_id = absent.endpoint_id();
        absent.shutdown().unwrap();
        let client = local_node(&dir_b, false);

        let addr = remote_endpoint_addr(&absent_id, None, &["127.0.0.1:9".to_string()]).unwrap();
        let dest = out.path().join("blob.bin");
        let token = CancellationToken::new();
        let canceller = token.clone();
        let started = std::time::Instant::now();
        let result = client.runtime().block_on(async {
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                canceller.cancel();
            });
            run_cancellable(
                fetch_to_path(
                    client.endpoint(),
                    Hash::new(b"x"),
                    addr,
                    &dest,
                    u64::MAX,
                    &token,
                    |_, _| {},
                ),
                &token,
                &client.shutdown_token(),
            )
            .await
        });
        assert!(
            matches!(result, Err(OperationError::Cancelled)),
            "expected Cancelled, got {result:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(leftover_files(out.path()).is_empty());

        client.shutdown().unwrap();
    }

    #[test]
    fn test_push_to_node_roundtrip() {
        let dir_a = tempdir().unwrap();
        let dir_b = tempdir().unwrap();

        // Create two nodes, relay disabled (local connection)
        let node_a = IrohNode::new(dir_a.path().to_path_buf(), false, None, false, true).unwrap();
        let node_b = IrohNode::new(dir_b.path().to_path_buf(), false, None, false, true).unwrap();

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
        let node = IrohNode::new(dir.path().to_path_buf(), true, None, false, true).unwrap();

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
        let node = IrohNode::new(dir.path().to_path_buf(), false, None, false, true).unwrap();

        let data = b"Hello, Iroh!";
        let ticket = node.put(data).unwrap();

        assert!(!ticket.is_empty());
        assert!(ticket.starts_with("blob")); // BlobTicket format

        node.shutdown().unwrap();
    }

    #[test]
    fn test_node_with_docs_enabled() {
        let dir = tempdir().unwrap();
        let node = IrohNode::new(dir.path().to_path_buf(), false, None, true, true).unwrap();

        assert!(node.is_docs_enabled());
        assert!(node.docs().is_some());

        node.shutdown().unwrap();
    }

    /// Download a blob from another node via ticket.
    /// Uses relay + n0 DNS discovery, so it needs network access. Run with:
    ///   cargo test test_get_cross_node_roundtrip -- --ignored --nocapture
    #[test]
    #[ignore]
    fn test_get_cross_node_roundtrip() {
        let dir_a = tempdir().unwrap();
        let dir_b = tempdir().unwrap();

        // Relay enabled so node B can discover node A via n0 DNS/relay
        let node_a = IrohNode::new(dir_a.path().to_path_buf(), true, None, false, true).unwrap();
        let node_b = IrohNode::new(dir_b.path().to_path_buf(), true, None, false, true).unwrap();

        let data = b"Cross-node get test data";
        let ticket = node_a.put(data).unwrap();

        let fetched = node_b.get_with_timeout(&ticket, 60_000).unwrap();
        assert_eq!(fetched, data);

        node_a.shutdown().unwrap();
        node_b.shutdown().unwrap();
    }
}
