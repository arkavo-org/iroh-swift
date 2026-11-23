//! IrohNode implementation wrapping iroh/iroh-blobs.
//!
//! Provides a minimal interface for blob storage and retrieval.

use crate::resolve::{
    DEFAULT_DHT_BOOTSTRAP, DHT_BOOTSTRAP_RESOLVE_BOUND, HostResolver, HostResolverConfig,
    RemoteTarget,
};
use anyhow::{Context, Result};
use bao_tree::io::BaoContentItem;
use iroh::address_lookup::AddrFilter;
use iroh::address_lookup::memory::MemoryLookup;
use iroh::endpoint::{Connection, ReadError, RelayMode, VarInt, presets};
use iroh::{Endpoint, EndpointAddr, RelayMap, RelayUrl, protocol::Router};
use iroh_blobs::api::blobs::BlobStatus;
use iroh_blobs::get::fsm::{
    AtBlobHeaderNextError, BlobContentNext, ConnectedNext, DecodeError, EndBlobNext,
};
use iroh_blobs::protocol::{ERR_INTERNAL, ERR_PERMISSION, GetRequest, PushRequest};
use iroh_blobs::provider::events::{EventMask, EventSender, RequestMode};
use iroh_blobs::{ALPN as BLOBS_ALPN, BlobsProtocol, Hash, store::fs::FsStore, ticket::BlobTicket};
use iroh_mainline_address_lookup::DhtAddressLookup;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

/// Settings beyond the positional arguments of [`IrohNode::new`].
#[derive(Debug, Clone, Default)]
pub struct NodeOptions {
    /// Resolve endpoint IDs on the BitTorrent Mainline DHT (pkarr), in addition
    /// to the preset's lookups, so a node can be dialled by ID alone.
    pub dht_lookup: bool,
    /// Also publish this node's own address to the DHT. Requires `dht_lookup`
    /// and a serving node (`serves_blobs`). Only the relay URL is published
    /// while relays are enabled; with relays disabled the direct addresses are
    /// (there is nothing else a peer could use).
    pub dht_publish: bool,
    /// Replace the DHT bootstrap nodes (`host:port`): a private DHT, or a
    /// local testnet. `None` keeps the public Mainline bootstrap nodes.
    pub dht_bootstrap: Option<Vec<String>>,
    /// How hostnames in direct addresses are resolved.
    pub host_resolver: HostResolverConfig,
}

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
    /// Whether the Mainline DHT address lookup is running. False when
    /// `dht_lookup` was not requested, or was but no bootstrap node resolved
    /// at creation (offline): recreate the node to get it.
    pub dht_active: bool,
}

/// Minimal Iroh node for blob operations.
///
/// Each node owns its own Tokio runtime to avoid conflicts with Swift's
/// concurrency model. All async operations are executed via `block_on`.
pub struct IrohNode {
    runtime: Runtime,
    endpoint: Endpoint,
    store: FsStore,
    router: Router,
    /// In-memory address lookup populated with `BlobTicket` direct IP and
    /// custom relay hints so `get` reaches peers without public discovery.
    memory_lookup: MemoryLookup,
    /// Whether `BlobsProtocol` is registered on `router` (determines whether
    /// `router.shutdown()` shuts down `store` or `IrohNode::shutdown()` must
    /// shut down `store` directly).
    serves_blobs: bool,
    /// Task draining the blobs provider's event channel (must be kept alive).
    event_handler: Option<tokio::task::JoinHandle<()>>,
    /// Parent of every spawned, cancellable operation (fetch, push, put, get, etc.).
    /// Cancelled first on shutdown so in-flight operations report
    /// [`OperationError::NodeShutDown`] instead of an opaque connection error.
    operations: CancellationToken,
    /// Resolves hostnames in direct addresses (DoH first, then the system resolver).
    host_resolver: HostResolver,
    /// Whether the DHT address lookup is installed (`dht_lookup` asked for it
    /// and at least one bootstrap node resolved).
    dht_active: bool,
}

impl IrohNode {
    /// Create a new Iroh node with persistent storage.
    ///
    /// # Arguments
    /// * `storage_path` - Directory for the blob store (created if doesn't exist)
    /// * `relay_enabled` - Whether to use relay servers
    /// * `custom_relay_url` - Optional custom relay URL (if None, uses n0's public relays)
    /// * `serves_blobs` - Whether to accept the iroh-blobs ALPN. When false the node
    ///   is client-only: it can still fetch and push, but peers cannot get blobs
    ///   from it (or push to it).
    /// * `accepts_pushes` - Whether peers may push blobs into this node's store.
    ///   Requires `serves_blobs`; when false, push requests are refused and
    ///   nothing is written (gets are still served).
    ///
    /// # Errors
    /// Fails if `accepts_pushes` is set without `serves_blobs`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new(
        storage_path: PathBuf,
        relay_enabled: bool,
        custom_relay_url: Option<String>,
        serves_blobs: bool,
        accepts_pushes: bool,
    ) -> Result<Self> {
        Self::new_with_options(
            storage_path,
            relay_enabled,
            custom_relay_url,
            serves_blobs,
            accepts_pushes,
            NodeOptions::default(),
        )
    }

    /// [`IrohNode::new`] with [`NodeOptions`].
    ///
    /// # Errors
    /// Fails if `accepts_pushes` is set without `serves_blobs`, or `dht_publish`
    /// without `dht_lookup` or without `serves_blobs`.
    pub fn new_with_options(
        storage_path: PathBuf,
        relay_enabled: bool,
        custom_relay_url: Option<String>,
        serves_blobs: bool,
        accepts_pushes: bool,
        options: NodeOptions,
    ) -> Result<Self> {
        let cancel = CancellationToken::new();
        Self::new_with_options_cancellable(
            storage_path,
            relay_enabled,
            custom_relay_url,
            serves_blobs,
            accepts_pushes,
            options,
            &cancel,
        )
        .map_err(|e| match e {
            OperationError::Failed(err) => err,
            other => anyhow::anyhow!("{other}"),
        })
    }

    /// Like [`IrohNode::new_with_options`], but stops promptly with
    /// [`OperationError::Cancelled`] when `cancel` fires (including during
    /// DHT bootstrap resolution and initial relay connection).
    pub fn new_with_options_cancellable(
        storage_path: PathBuf,
        relay_enabled: bool,
        custom_relay_url: Option<String>,
        serves_blobs: bool,
        accepts_pushes: bool,
        options: NodeOptions,
        cancel: &CancellationToken,
    ) -> Result<Self, OperationError> {
        if cancel.is_cancelled() {
            return Err(OperationError::Cancelled);
        }
        if accepts_pushes && !serves_blobs {
            return Err(anyhow::anyhow!("accepts_pushes requires serves_blobs").into());
        }
        if options.dht_publish && !options.dht_lookup {
            return Err(anyhow::anyhow!("dht_publish requires dht_lookup").into());
        }
        if options.dht_publish && !serves_blobs {
            return Err(anyhow::anyhow!("dht_publish requires serves_blobs").into());
        }
        let host_resolver = HostResolver::new(HostResolverConfig {
            fall_through_to_lookup: false,
            ..options.host_resolver.clone()
        });

        // Create dedicated runtime for this node
        let runtime = Runtime::new().context("Failed to create Tokio runtime")?;

        let memory_lookup = MemoryLookup::new();
        let init_result = runtime.block_on(async {
            if cancel.is_cancelled() {
                return Err(OperationError::Cancelled);
            }

            // Let `FsStore::load` finish rather than dropping it mid-flight inside
            // `select!`: dropping an in-flight load only triggers background
            // shutdown, which can leave redb's lock held after `Cancelled` returns.
            let store = FsStore::load(&storage_path)
                .await
                .context("Failed to load blob store")?;

            // Helper to shut down `store` cleanly if initialization fails or is cancelled.
            async fn cleanup_store(store: FsStore) {
                let _ = tokio::time::timeout(Duration::from_secs(5), store.shutdown()).await;
                drop(store);
            }

            if cancel.is_cancelled() {
                cleanup_store(store).await;
                return Err(OperationError::Cancelled);
            }

            // Build endpoint with relay configuration and in-memory ticket lookup
            let mut builder =
                Endpoint::builder(presets::N0).address_lookup(memory_lookup.clone());
            if !relay_enabled {
                builder = builder.relay_mode(RelayMode::Disabled);
            } else if let Some(url) = custom_relay_url {
                // Parse and use custom relay
                let relay_url: RelayUrl = match url.parse().context("Invalid relay URL") {
                    Ok(u) => u,
                    Err(e) => {
                        cleanup_store(store).await;
                        return Err(e.into());
                    }
                };
                let relay_map = RelayMap::from(relay_url);
                builder = builder.relay_mode(RelayMode::Custom(relay_map));
            }
            // else: n0 public relays are default when relay_enabled=true

            // n0-mainline resolves its bootstrap hostnames with blocking DNS on the
            // thread that builds the DHT, once, and drops failures silently. So
            // resolve them here instead (bounded, DoH first, then the OS) and hand
            // it IP addresses. If none resolve the DHT would stay empty for the
            // node's whole life, so start without it (see `NodeInfo::dht_active`).
            let mut dht_active = false;
            if options.dht_lookup {
                let bootstrap = match &options.dht_bootstrap {
                    Some(bootstrap) => bootstrap.clone(),
                    None => {
                        let resolved = tokio::select! {
                            biased;
                            _ = cancel.cancelled() => {
                                cleanup_store(store).await;
                                return Err(OperationError::Cancelled);
                            }
                            res = host_resolver
                                .resolve_bootstrap(&DEFAULT_DHT_BOOTSTRAP, DHT_BOOTSTRAP_RESOLVE_BOUND) => res,
                        };
                        resolved
                    }
                };
                if bootstrap.is_empty() {
                    tracing::warn!(
                        "no DHT bootstrap node resolved (offline?): starting without DHT \
                         lookup; recreate the node once the network is up to get it"
                    );
                } else {
                    let mut dht = n0_mainline::Dht::builder();
                    dht.bootstrap(&bootstrap);
                    let mut lookup = DhtAddressLookup::builder().dht_builder(dht);
                    if options.dht_publish {
                        if !relay_enabled {
                            // Without a relay URL there is nothing to publish under
                            // the default relay-only filter; the node is only
                            // reachable on its direct addresses.
                            lookup = lookup.addr_filter(AddrFilter::unfiltered());
                        }
                    } else {
                        lookup = lookup.no_publish();
                    }
                    builder = builder.address_lookup(lookup);
                    dht_active = true;
                }
            }

            let endpoint = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    cleanup_store(store).await;
                    return Err(OperationError::Cancelled);
                }
                res = builder.bind() => match res.context("Failed to bind endpoint") {
                    Ok(ep) => ep,
                    Err(e) => {
                        cleanup_store(store).await;
                        return Err(e.into());
                    }
                }
            };

            // Wait for relay connection if enabled (with timeout to avoid hanging in CI)
            if relay_enabled {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        endpoint.close().await;
                        cleanup_store(store).await;
                        return Err(OperationError::Cancelled);
                    }
                    _ = tokio::time::timeout(Duration::from_secs(10), endpoint.online()) => {}
                }
            }

            if cancel.is_cancelled() {
                endpoint.close().await;
                cleanup_store(store).await;
                return Err(OperationError::Cancelled);
            }

            // Set up the blobs protocol handler, unless the node is client-only.
            // Fetching (`store.remote()`, the downloader) and pushing only need the
            // store and the endpoint, not the handler.
            let (blobs, event_handler) = if serves_blobs {
                // Gets are always served. Pushes write to this node's store, so they
                // are refused (`Disabled`, the iroh-blobs default) unless the app
                // opts in; `Notify` processes them without asking a handler.
                let mask = EventMask {
                    push: if accepts_pushes {
                        RequestMode::Notify
                    } else {
                        RequestMode::Disabled
                    },
                    ..EventMask::DEFAULT
                };
                let (event_sender, mut event_rx) = EventSender::channel(64, mask);

                // Nothing here needs an answer (no Intercept modes), but the provider
                // awaits each send (push notifications, connection-closed events), so
                // drain the channel to keep it from filling up.
                let event_handler =
                    tokio::spawn(async move { while event_rx.recv().await.is_some() {} });

                let blobs = BlobsProtocol::new(&store, Some(event_sender));
                (Some(blobs), Some(event_handler))
            } else {
                (None, None)
            };

            // Build router with all protocols. A client-only node registers no
            // blobs ALPN, so incoming blobs connections fail the TLS/ALPN handshake.
            let mut router_builder = Router::builder(endpoint.clone());

            if let Some(blobs) = blobs {
                router_builder = router_builder.accept(BLOBS_ALPN, blobs);
            }

            let router = router_builder.spawn();

            Ok::<_, OperationError>((endpoint, store, router, event_handler, dht_active))
        });

        let (endpoint, store, router, event_handler, dht_active) = match init_result {
            Ok(parts) => parts,
            Err(err) => {
                runtime.shutdown_timeout(Duration::from_secs(2));
                return Err(err);
            }
        };

        Ok(Self {
            runtime,
            endpoint,
            store,
            router,
            memory_lookup,
            serves_blobs,
            event_handler,
            operations: CancellationToken::new(),
            // With the DHT active, unresolvable hostnames must not stop a dial by ID.
            host_resolver: host_resolver.with_fall_through_to_lookup(dht_active),
            dht_active,
        })
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

    /// The resolver for hostnames in direct addresses (cheap to clone).
    pub fn host_resolver(&self) -> &HostResolver {
        &self.host_resolver
    }

    /// In-memory address lookup populated with `BlobTicket` hints (cheap to clone).
    pub fn memory_lookup(&self) -> &MemoryLookup {
        &self.memory_lookup
    }

    /// Whether the Mainline DHT address lookup is running on this node.
    pub fn dht_active(&self) -> bool {
        self.dht_active
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
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn put(&self, data: &[u8]) -> Result<String> {
        self.put_with_timeout(data, 0)
    }

    /// Download bytes from a ticket.
    ///
    /// This fetches the blob from the remote peer specified in the ticket.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn get(&self, ticket_str: &str) -> Result<Vec<u8>> {
        self.get_with_timeout(ticket_str, 0)
    }

    /// Download bytes from a ticket with progress reporting.
    ///
    /// The progress callback is called with (downloaded, total) byte counts.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn get_with_progress<F>(&self, ticket_str: &str, on_progress: F) -> Result<Vec<u8>>
    where
        F: FnMut(u64, u64),
    {
        let cancel = CancellationToken::new();
        self.runtime
            .block_on(get_from_ticket(
                &self.endpoint,
                &self.store,
                &self.memory_lookup,
                ticket_str,
                u64::MAX,
                0,
                &cancel,
                on_progress,
            ))
            .map_err(operation_error_to_anyhow)
    }

    /// Add bytes to the blob store with an optional timeout.
    ///
    /// # Arguments
    /// * `data` - The bytes to store
    /// * `timeout_ms` - Timeout in milliseconds (0 = no timeout)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn put_with_timeout(&self, data: &[u8], timeout_ms: u64) -> Result<String> {
        self.runtime
            .block_on(put_bytes(&self.endpoint, &self.store, data, timeout_ms))
            .map_err(operation_error_to_anyhow)
    }

    /// Download bytes from a ticket with an optional timeout.
    ///
    /// # Arguments
    /// * `ticket_str` - The ticket string
    /// * `timeout_ms` - Timeout in milliseconds (0 = no timeout)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn get_with_timeout(&self, ticket_str: &str, timeout_ms: u64) -> Result<Vec<u8>> {
        let cancel = CancellationToken::new();
        self.runtime
            .block_on(get_from_ticket(
                &self.endpoint,
                &self.store,
                &self.memory_lookup,
                ticket_str,
                u64::MAX,
                timeout_ms,
                &cancel,
                |_, _| {},
            ))
            .map_err(operation_error_to_anyhow)
    }

    /// Push data to a remote node.
    ///
    /// Resolves the target first, adds the data to the local store, connects to
    /// the remote node, and pushes the blob. Returns the blob hash as a hex string.
    ///
    /// # Arguments
    /// * `remote_node_id_hex` - The remote node's ID as a hex string
    /// * `relay_url` - Optional relay URL for routing the connection
    /// * `data` - The bytes to push
    /// * `direct_addrs` - Optional direct addresses, `IP:port` or `host:port`
    ///   (hostnames resolve through DoH, then the system resolver)
    pub fn push_to_node(
        &self,
        remote_node_id_hex: &str,
        relay_url: Option<&str>,
        data: &[u8],
        direct_addrs: &[String],
    ) -> Result<String> {
        self.push_to_node_with_timeout(remote_node_id_hex, relay_url, data, direct_addrs, 0)
    }

    /// Push data to a remote node with an optional timeout.
    ///
    /// # Arguments
    /// * `remote_node_id_hex` - The remote node's ID as a hex string
    /// * `relay_url` - Optional relay URL for routing the connection
    /// * `data` - The bytes to push
    /// * `direct_addrs` - Optional direct addresses, `IP:port` or `host:port`
    ///   (hostnames resolve through DoH, then the system resolver)
    /// * `timeout_ms` - Timeout in milliseconds (0 = no timeout)
    pub fn push_to_node_with_timeout(
        &self,
        remote_node_id_hex: &str,
        relay_url: Option<&str>,
        data: &[u8],
        direct_addrs: &[String],
        timeout_ms: u64,
    ) -> Result<String> {
        let target = RemoteTarget::parse(remote_node_id_hex, relay_url, direct_addrs)?;
        self.runtime
            .block_on(push_bytes(
                &self.endpoint,
                &self.store,
                &self.host_resolver,
                target,
                data,
                timeout_ms,
            ))
            .map(|hash| hex::encode(hash.as_bytes()))
            .map_err(operation_error_to_anyhow)
    }

    /// Get information about this node.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn info(&self) -> Result<NodeInfo> {
        Ok(node_info_from_endpoint(&self.endpoint, self.dht_active))
    }

    /// Gracefully shut down the node.
    ///
    /// This ensures all pending writes are flushed to disk, including for
    /// client-only nodes where `BlobsProtocol` is not registered on the router.
    /// Uses timeouts to prevent hanging while surfacing shutdown errors/timeouts
    /// instead of silently discarding them.
    pub fn shutdown(self) -> Result<()> {
        // Destructure to control drop order: drop all node resources first,
        // then shut down the runtime last. This prevents Drop impls on
        // endpoint/store/etc from trying to use an already-shut-down runtime.
        let IrohNode {
            runtime,
            router,
            endpoint,
            store,
            memory_lookup,
            serves_blobs,
            event_handler,
            operations,
            host_resolver,
            dht_active: _,
        } = self;
        drop(host_resolver);
        drop(memory_lookup);

        // Stop in-flight spawned operations first, before the endpoint goes
        // away: each reports NodeShutDown (a fetch also removes its temp file).
        operations.cancel();

        // Actively abort the provider event drain task rather than detaching it.
        if let Some(handle) = event_handler {
            handle.abort();
        }

        let result = runtime.block_on(async {
            let mut first_err: Option<anyhow::Error> = None;

            let router_ok = match tokio::time::timeout(Duration::from_secs(5), router.shutdown())
                .await
            {
                Ok(Ok(())) => true,
                Ok(Err(e)) => {
                    first_err = Some(anyhow::Error::new(e).context("Failed to shutdown router"));
                    false
                }
                Err(_) => {
                    first_err = Some(anyhow::anyhow!("Timed out shutting down router"));
                    false
                }
            };

            // When `serves_blobs == false`, BlobsProtocol is not registered on
            // the router so `router.shutdown()` never flushes/closes `store`.
            // Also attempt store shutdown if router shutdown failed/timed out.
            if !serves_blobs || !router_ok {
                match tokio::time::timeout(Duration::from_secs(5), store.shutdown()).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        if first_err.is_none() {
                            first_err = Some(
                                anyhow::Error::new(e).context("Failed to shutdown blob store"),
                            );
                        }
                    }
                    Err(_) => {
                        if first_err.is_none() {
                            first_err = Some(anyhow::anyhow!("Timed out shutting down blob store"));
                        }
                    }
                }
            }

            match first_err {
                Some(e) => Err(e),
                None => Ok(()),
            }
        });

        // Drop all node resources before shutting down the runtime.
        drop(store);
        drop(endpoint);

        // Explicitly shut down the runtime with a timeout so that
        // background tasks don't block process exit.
        runtime.shutdown_timeout(Duration::from_secs(2));

        result
    }
}

fn operation_error_to_anyhow(e: OperationError) -> anyhow::Error {
    match e {
        OperationError::Failed(err) => err,
        other => anyhow::anyhow!("{other}"),
    }
}

/// Snapshot of [`NodeInfo`] from an endpoint without blocking on a runtime.
pub fn node_info_from_endpoint(endpoint: &Endpoint, dht_active: bool) -> NodeInfo {
    let node_id = endpoint.id().to_string();
    let addr = endpoint.addr();
    let relay_url = addr.relay_urls().next().map(|url| url.to_string());
    let is_connected = relay_url.is_some() || addr.ip_addrs().next().is_some();
    let direct_addrs = addr.ip_addrs().map(|a| a.to_string()).collect();

    NodeInfo {
        node_id,
        relay_url,
        is_connected,
        direct_addrs,
        dht_active,
    }
}

/// Add bytes to `store` and return a shareable `BlobTicket` string.
/// `timeout_ms == 0` means no timeout.
pub async fn put_bytes(
    endpoint: &Endpoint,
    store: &FsStore,
    data: &[u8],
    timeout_ms: u64,
) -> Result<String, OperationError> {
    let work = async {
        let tag = store
            .add_slice(data)
            .await
            .context("Failed to add bytes to store")?;
        let addr = endpoint.addr();
        let ticket = BlobTicket::new(addr, tag.hash, tag.format);
        Ok::<_, OperationError>(ticket.to_string())
    };
    if timeout_ms == 0 {
        work.await
    } else {
        tokio::time::timeout(Duration::from_millis(timeout_ms), work)
            .await
            .map_err(|_| OperationError::TimedOut)?
    }
}

/// Download bytes from a `BlobTicket`, populating `memory_lookup` with the
/// ticket's direct IP and custom relay hints and enforcing `max_size` before
/// reading any body or allocating disk space.
///
/// `timeout_ms == 0` means no timeout; `max_size == u64::MAX` means no limit.
#[allow(clippy::too_many_arguments)]
pub async fn get_from_ticket(
    endpoint: &Endpoint,
    store: &FsStore,
    memory_lookup: &MemoryLookup,
    ticket_str: &str,
    max_size: u64,
    timeout_ms: u64,
    cancel: &CancellationToken,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<Vec<u8>, OperationError> {
    let ticket: BlobTicket = ticket_str.parse().context("Failed to parse ticket")?;

    let work = async {
        // Fast path: already complete in local store. Check size before reading into RAM.
        if let Ok(BlobStatus::Complete { size }) = store.blobs().status(ticket.hash()).await {
            if size > max_size {
                return Err(OperationError::SizeLimitExceeded(size));
            }
            let bytes = store
                .get_bytes(ticket.hash())
                .await
                .context("Failed to read bytes from store")?;
            let actual = bytes.len() as u64;
            if actual > max_size {
                return Err(OperationError::SizeLimitExceeded(actual));
            }
            on_progress(actual, actual);
            return Ok(Vec::from(bytes));
        }

        if ticket.addr().id == endpoint.id() {
            return Err(OperationError::NotFound);
        }

        // Preserve the ticket's direct IP and custom relay hints so `endpoint` can
        // dial the peer directly even with public discovery/relays disabled.
        if !ticket.addr().is_empty() {
            memory_lookup.add_endpoint_info(ticket.addr().clone());
        }

        // Dial by endpoint ID; `memory_lookup` provides the ticket's direct IP
        // and custom relay hints alongside any other configured lookups.
        let conn = endpoint
            .connect(ticket.addr().id, BLOBS_ALPN)
            .await
            .context("Failed to connect to remote node")?;

        let result =
            get_bytes_on_connection(&conn, ticket.hash(), max_size, cancel, &mut on_progress).await;

        conn.close(VarInt::from_u32(0), b"done");
        let buf = result?;

        if cancel.is_cancelled() {
            return Err(OperationError::Cancelled);
        }

        // Cache verified blob (already bounded by `max_size`) in local store using
        // a temp tag (not a permanent auto-tag) so `tagBlob`/`untagBlob` controls pinning.
        let _temp_tag = store
            .blobs()
            .add_slice(&buf)
            .temp_tag()
            .await
            .context("Failed to store downloaded blob")?;

        Ok(buf)
    };

    if timeout_ms == 0 {
        work.await
    } else {
        tokio::time::timeout(Duration::from_millis(timeout_ms), work)
            .await
            .map_err(|_| OperationError::TimedOut)?
    }
}

async fn get_bytes_on_connection(
    conn: &Connection,
    hash: Hash,
    max_size: u64,
    cancel: &CancellationToken,
    on_progress: &mut impl FnMut(u64, u64),
) -> Result<Vec<u8>, OperationError> {
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

    // Enforce max_size from the size header before allocating body RAM or disk.
    let (mut content, size) = root.next().next().await.map_err(header_error)?;
    if size > max_size {
        return Err(OperationError::SizeLimitExceeded(size));
    }
    let expected_len =
        usize::try_from(size).map_err(|_| OperationError::SizeLimitExceeded(size))?;

    // Cap initial reservation so a peer announcing a large size within `max_size`
    // and disconnecting immediately cannot force an upfront allocation.
    let mut buf = Vec::with_capacity(expected_len.min(64 * 1024));

    on_progress(0, size);
    let mut downloaded: u64 = 0;
    let mut reported: u64 = 0;

    let end = loop {
        match content.next().await {
            BlobContentNext::More((next, item)) => {
                content = next;
                match item.map_err(decode_error)? {
                    BaoContentItem::Parent(_) => {}
                    BaoContentItem::Leaf(leaf) => {
                        let len = leaf.data.len() as u64;
                        let leaf_end = leaf.offset.saturating_add(len);
                        if leaf.offset != downloaded {
                            return Err(anyhow::anyhow!(
                                "Unexpected non-sequential blob leaf offset {} (expected {})",
                                leaf.offset,
                                downloaded
                            )
                            .into());
                        }
                        if leaf_end > size {
                            return Err(anyhow::anyhow!(
                                "Remote sent data past the announced size ({leaf_end} > {size})"
                            )
                            .into());
                        }
                        downloaded = downloaded.saturating_add(len);
                        if downloaded > max_size {
                            return Err(OperationError::SizeLimitExceeded(downloaded));
                        }
                        buf.extend_from_slice(&leaf.data);

                        if downloaded == size || downloaded - reported >= FETCH_PROGRESS_STEP {
                            on_progress(downloaded, size);
                            reported = downloaded;
                        }
                        if cancel.is_cancelled() {
                            return Err(OperationError::Cancelled);
                        }
                    }
                }
            }
            BlobContentNext::Done(end) => break end,
        }
    };

    if downloaded != size || buf.len() != expected_len {
        return Err(anyhow::anyhow!("Remote finished after {downloaded} of {size} bytes").into());
    }

    if let EndBlobNext::Closing(closing) = end.next() {
        let _ = closing.next().await;
    }

    if reported != size {
        on_progress(size, size);
    }

    Ok(buf)
}

/// Push a blob that is already in `store` to a remote node, returning the live
/// `Connection` guard so the caller can keep it open briefly outside any
/// timeout/cancellation scope (via [`await_push_grace`]).
async fn push_hash_transfer(
    endpoint: &Endpoint,
    store: &FsStore,
    hash: Hash,
    endpoint_addr: EndpointAddr,
) -> Result<Connection> {
    // Connect to remote node
    let conn = endpoint
        .connect(endpoint_addr, BLOBS_ALPN)
        .await
        .context("Failed to connect to remote node")?;

    // Keep a connection handle alive — execute_push takes ownership but
    // Connection is Clone (ref-counted). The server needs time to
    // accept_bi and process the push stream after execute_push returns.
    let conn_guard = conn.clone();

    // Push the blob
    let request = PushRequest::from(GetRequest::blob(hash));
    store
        .remote()
        .execute_push(conn, request)
        .await
        .context("Failed to push blob to remote node")?;

    Ok(conn_guard)
}

/// Keep a pushed connection alive briefly so the server can finish `accept_bi`
/// and commit small blobs before the connection drops.
///
/// Runs OUTSIDE the caller's timeout and cancellation error scopes so an
/// already-succeeded push never fails with a false `TimedOut` or `Cancelled`.
/// If node shutdown interrupts this wait before the peer closes or the grace
/// period elapses, returns [`OperationError::NodeShutDown`] because closing the
/// endpoint may interrupt delivery before the peer commits the blob.
pub async fn await_push_grace(
    conn: Connection,
    cancel: Option<&CancellationToken>,
    shutdown: Option<&CancellationToken>,
) -> Result<(), OperationError> {
    let grace = Duration::from_millis(500);
    let cancelled = async {
        match cancel {
            Some(token) => token.cancelled().await,
            None => std::future::pending().await,
        }
    };
    let shutting_down = async {
        match shutdown {
            Some(token) => token.cancelled().await,
            None => std::future::pending().await,
        }
    };

    tokio::select! {
        biased;
        _ = shutting_down => Err(OperationError::NodeShutDown),
        _ = conn.closed() => Ok(()),
        _ = tokio::time::sleep(grace) => Ok(()),
        _ = cancelled => {
            // Transfer already completed: return immediately to the caller
            // while holding the connection open briefly in the background.
            tokio::spawn(async move {
                tokio::select! {
                    _ = conn.closed() => {}
                    _ = tokio::time::sleep(grace) => {}
                }
            });
            Ok(())
        }
    }
}

/// Why a spawned operation (fetch, file push) did not complete.
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
    /// The caller's time limit elapsed.
    TimedOut,
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
            OperationError::TimedOut => write!(f, "operation timed out"),
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

/// Import the file at `path` into the store (streamed, copied into the store)
/// and push it to a remote node, returning `(hash, conn)` so the caller can run
/// [`await_push_grace`] outside its cancellation scope.
pub async fn push_path_transfer(
    endpoint: &Endpoint,
    store: &FsStore,
    path: &Path,
    remote: EndpointAddr,
) -> Result<(Hash, Connection), OperationError> {
    if !path.is_absolute() {
        return Err(anyhow::anyhow!("File path must be absolute: {}", path.display()).into());
    }
    let tag = store
        .add_path(path)
        .await
        .with_context(|| format!("Failed to import {} into the store", path.display()))?;
    let conn = push_hash_transfer(endpoint, store, tag.hash, remote).await?;
    Ok((tag.hash, conn))
}

/// Import the file at `path` into the store (streamed, copied into the store)
/// and push it to a remote node. Returns the blob hash.
#[cfg_attr(not(test), allow(dead_code))]
pub async fn push_path(
    endpoint: &Endpoint,
    store: &FsStore,
    path: &Path,
    remote: EndpointAddr,
) -> Result<Hash, OperationError> {
    let (hash, conn) = push_path_transfer(endpoint, store, path, remote).await?;
    await_push_grace(conn, None, None).await?;
    Ok(hash)
}

/// Resolve `target`, add `data` to `store`, and push it to the remote node,
/// returning `(hash, conn)` so [`await_push_grace`] runs outside `timeout_ms`
/// and caller cancellation.
pub async fn push_bytes_transfer(
    endpoint: &Endpoint,
    store: &FsStore,
    resolver: &HostResolver,
    target: RemoteTarget,
    data: &[u8],
    timeout_ms: u64,
) -> Result<(Hash, Connection), OperationError> {
    let work = async {
        let remote = resolver.resolve_target(target).await?;
        let tag = store
            .add_slice(data)
            .await
            .context("Failed to add bytes to store")?;
        let conn = push_hash_transfer(endpoint, store, tag.hash, remote).await?;
        Ok::<_, OperationError>((tag.hash, conn))
    };
    if timeout_ms == 0 {
        work.await
    } else {
        tokio::time::timeout(Duration::from_millis(timeout_ms), work)
            .await
            .map_err(|_| OperationError::TimedOut)?
    }
}

/// Add `data` to the store and push it to a remote node, resolving the target's
/// hostnames first (so a cancel during DNS also stops the push). `timeout_ms` of 0
/// means no timeout; it covers DNS, import and transfer (not the post-transfer
/// connection grace wait). Returns the blob hash.
pub async fn push_bytes(
    endpoint: &Endpoint,
    store: &FsStore,
    resolver: &HostResolver,
    target: RemoteTarget,
    data: &[u8],
    timeout_ms: u64,
) -> Result<Hash, OperationError> {
    let (hash, conn) =
        push_bytes_transfer(endpoint, store, resolver, target, data, timeout_ms).await?;
    await_push_grace(conn, None, None).await?;
    Ok(hash)
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

    /// The address of a node from literal `IP:port` entries (no DNS).
    fn remote_endpoint_addr(
        id: &str,
        relay_url: Option<&str>,
        direct_addrs: &[String],
    ) -> Result<EndpointAddr> {
        let target = RemoteTarget::parse(id, relay_url, direct_addrs)?;
        assert!(target.hosts.is_empty(), "literal addresses only");
        let mut addr = EndpointAddr::from(target.id);
        if let Some(relay) = target.relay {
            addr = addr.with_relay_url(relay);
        }
        for socket in target.sockets {
            addr = addr.with_ip_addr(socket);
        }
        Ok(addr)
    }

    fn local_node(dir: &tempfile::TempDir, serves_blobs: bool) -> IrohNode {
        IrohNode::new(dir.path().to_path_buf(), false, None, serves_blobs, false).unwrap()
    }

    /// A serving node that accepts pushes.
    fn push_receiver(dir: &tempfile::TempDir) -> IrohNode {
        IrohNode::new(dir.path().to_path_buf(), false, None, true, true).unwrap()
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
    fn test_push_file_from_client_only_node_then_fetch_back() {
        let (dir_a, dir_b, out) = (tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap());
        let server = push_receiver(&dir_a);
        let client = local_node(&dir_b, false);

        let data = test_bytes(1024 * 1024 + 7);
        let source = out.path().join("source.tdf");
        std::fs::write(&source, &data).unwrap();

        let addr =
            remote_endpoint_addr(&server.endpoint_id(), None, &direct_addrs(&server)).unwrap();
        let hash = client
            .runtime()
            .block_on(push_path(client.endpoint(), client.store(), &source, addr))
            .unwrap();
        assert_eq!(hash.to_hex().to_string(), blake3_hex(&data));

        let has = server.runtime().block_on(server.store().has(hash)).unwrap();
        assert!(has, "server should have the pushed blob");

        let dest = out.path().join("fetched.tdf");
        let (result, _) = fetch(&client, &server, hash, &dest, u64::MAX);
        assert_eq!(result.unwrap(), data.len() as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), data);

        client.shutdown().unwrap();
        server.shutdown().unwrap();
    }

    /// A serving node with the default config (`accepts_pushes: false`) refuses
    /// pushes: nothing lands in its store, whatever the size.
    ///
    /// The pusher is not told reliably. iroh-blobs' push client stops reading its
    /// side of the stream at once, so it never sees the provider's
    /// `ERR_PERMISSION` reset; it only fails if the provider drops the stream
    /// (STOP_SENDING) while the client is still writing. A blob small enough to
    /// be written in one go reports success; one still being written when the
    /// STOP_SENDING arrives fails with "sending stopped by peer" (a race for
    /// mid-size blobs, 1 MiB already fails on localhost; certain once the client
    /// waits for flow-control credit). If the small push starts failing, the
    /// pusher now learns about refusals: update the docs that say it may not.
    #[test]
    fn test_serving_node_refuses_pushes_by_default() {
        let (dir_a, dir_b, out) = (tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap());
        let server = local_node(&dir_a, true);
        let client = local_node(&dir_b, false);
        let server_id = server.endpoint_id();
        let addrs = direct_addrs(&server);

        let small = b"refused push";
        let small_result = client.push_to_node(&server_id, None, small, &addrs);
        match small_result {
            Ok(hash) => assert_eq!(hash, blake3_hex(small)),
            Err(error) => assert!(
                format!("{error:#}").contains("stopped by peer"),
                "unexpected error for small refused push: {error:#}"
            ),
        }

        let large = test_bytes(2 * 1024 * 1024 + 3);
        let source = out.path().join("large.bin");
        std::fs::write(&source, &large).unwrap();
        let addr = remote_endpoint_addr(&server_id, None, &addrs).unwrap();
        let large_result =
            client
                .runtime()
                .block_on(push_path(client.endpoint(), client.store(), &source, addr));
        match large_result {
            Err(OperationError::Failed(error)) => assert!(
                format!("{error:#}").contains("stopped by peer"),
                "unexpected error: {error:#}"
            ),
            other => panic!("expected the large refused push to fail, got {other:?}"),
        }

        for data in [&small[..], &large[..]] {
            let hash: Hash = blake3_hex(data).parse().unwrap();
            let has = server.runtime().block_on(server.store().has(hash)).unwrap();
            assert!(!has, "a refused push must not write to the server's store");
        }

        client.shutdown().unwrap();
        server.shutdown().unwrap();
    }

    #[test]
    fn test_accepts_pushes_requires_serves_blobs() {
        let dir = tempdir().unwrap();
        let result = IrohNode::new(dir.path().to_path_buf(), false, None, false, true);
        let error = result
            .err()
            .expect("accepts_pushes without serves_blobs must fail");
        assert!(
            error
                .to_string()
                .contains("accepts_pushes requires serves_blobs"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn test_push_to_node_roundtrip() {
        let dir_a = tempdir().unwrap();
        let dir_b = tempdir().unwrap();

        // Create two nodes, relay disabled (local connection); B accepts pushes.
        let node_a = local_node(&dir_a, true);
        let node_b = push_receiver(&dir_b);

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
        assert_eq!(hash_hex, blake3_hex(data));

        let hash: Hash = hash_hex.parse().unwrap();
        let has = node_b.runtime().block_on(node_b.store().has(hash)).unwrap();
        assert!(has, "node B should have the pushed blob");

        node_a.shutdown().unwrap();
        node_b.shutdown().unwrap();
    }

    /// A node whose DNS-over-HTTPS list points at a closed local port, so the
    /// DoH stage answers nothing (fast) and every lookup reaches the system
    /// resolver, with no dependence on the internet.
    fn node_with_dead_doh(dir: &tempfile::TempDir, serves_blobs: bool) -> IrohNode {
        use crate::resolve::HostResolverConfig;
        use iroh::dns::NameserverConfig;
        let options = NodeOptions {
            host_resolver: HostResolverConfig {
                doh_nameservers: vec![
                    NameserverConfig::https("127.0.0.1".parse().unwrap())
                        .with_port(1)
                        .with_tls_server_name("doh.invalid"),
                ],
                doh_timeout: Duration::from_millis(500),
                system_timeout: Duration::from_secs(3),
                ..HostResolverConfig::default()
            },
            ..NodeOptions::default()
        };
        IrohNode::new_with_options(
            dir.path().to_path_buf(),
            false,
            None,
            serves_blobs,
            false,
            options,
        )
        .unwrap()
    }

    /// Port of the server's first IPv4 direct address.
    fn server_port(server: &IrohNode) -> u16 {
        direct_addrs(server)
            .iter()
            .filter_map(|a| a.parse::<std::net::SocketAddr>().ok())
            .find(|a| a.is_ipv4())
            .expect("an IPv4 direct address")
            .port()
    }

    #[test]
    fn test_localhost_resolves_via_system_when_doh_is_unreachable() {
        let (dir, dir_server) = (tempdir().unwrap(), tempdir().unwrap());
        let client = node_with_dead_doh(&dir, false);
        let server = local_node(&dir_server, true);
        let port = server_port(&server);

        let target =
            RemoteTarget::parse(&server.endpoint_id(), None, &[format!("localhost:{port}")])
                .unwrap();
        let start = std::time::Instant::now();
        let addr = client
            .runtime()
            .block_on(client.host_resolver().resolve_target(target))
            .unwrap();
        assert!(start.elapsed() < Duration::from_secs(3));
        let ips: Vec<_> = addr.ip_addrs().collect();
        assert!(!ips.is_empty());
        assert!(
            ips.iter().all(|a| a.ip().is_loopback() && a.port() == port),
            "{ips:?}"
        );

        client.shutdown().unwrap();
        server.shutdown().unwrap();
    }

    /// The node dials a serving node given only `localhost:<port>`, the way an
    /// app dials `iroh.arkavo.net:11204`.
    #[test]
    fn test_fetch_from_serving_node_by_hostname() {
        let (dir_a, dir_b, out) = (tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap());
        let server = local_node(&dir_a, true);
        let client = node_with_dead_doh(&dir_b, false);

        let data = test_bytes(300_000);
        let ticket = server.put(&data).unwrap();
        let hash = ticket.parse::<BlobTicket>().unwrap().hash();

        let target = RemoteTarget::parse(
            &server.endpoint_id(),
            None,
            &[format!("localhost:{}", server_port(&server))],
        )
        .unwrap();
        let dest = out.path().join("by-host.bin");
        let token = CancellationToken::new();
        let result = client.runtime().block_on(async {
            let addr = client.host_resolver().resolve_target(target).await?;
            fetch_to_path(
                client.endpoint(),
                hash,
                addr,
                &dest,
                u64::MAX,
                &token,
                |_, _| {},
            )
            .await
        });
        assert_eq!(result.unwrap(), data.len() as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), data);

        client.shutdown().unwrap();
        server.shutdown().unwrap();
    }

    #[test]
    fn test_push_to_node_by_hostname() {
        let (dir_a, dir_b) = (tempdir().unwrap(), tempdir().unwrap());
        let server = push_receiver(&dir_a);
        let client = node_with_dead_doh(&dir_b, false);

        let data = test_bytes(2_000);
        let hash = client
            .push_to_node(
                &server.endpoint_id(),
                None,
                &data,
                &[format!("localhost:{}", server_port(&server))],
            )
            .unwrap();
        assert_eq!(hash, blake3_hex(&data));
        let parsed = Hash::from_bytes(hex::decode(&hash).unwrap().try_into().unwrap());
        assert!(
            server
                .runtime()
                .block_on(server.store().has(parsed))
                .unwrap()
        );

        client.shutdown().unwrap();
        server.shutdown().unwrap();
    }

    #[test]
    fn test_bad_host_fails_cleanly_and_quickly() {
        let (dir_a, dir_b) = (tempdir().unwrap(), tempdir().unwrap());
        let server = local_node(&dir_a, true);
        let client = node_with_dead_doh(&dir_b, false);

        let start = std::time::Instant::now();
        let err = client
            .push_to_node(
                &server.endpoint_id(),
                None,
                b"x",
                &["no-such-host.invalid:11204".to_string()],
            )
            .unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("Could not resolve"), "{message}");
        assert!(message.contains("no-such-host.invalid:11204"), "{message}");
        assert!(start.elapsed() < Duration::from_secs(5));

        // A malformed entry is rejected before any DNS or store work.
        let err = client
            .push_to_node(&server.endpoint_id(), None, b"x", &["host:0".to_string()])
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("Invalid direct address"),
            "{err:#}"
        );

        client.shutdown().unwrap();
        server.shutdown().unwrap();
    }

    /// Needs internet: the default public DoH resolvers answer for a real name.
    /// `cargo test -- --ignored test_default_doh`
    #[test]
    #[ignore = "needs internet"]
    fn test_default_doh_resolves_public_name() {
        let resolver = HostResolver::new(HostResolverConfig::default());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let ips = rt.block_on(resolver.resolve_host("one.one.one.one"));
        assert!(ips.iter().any(|ip| ip.to_string() == "1.1.1.1"), "{ips:?}");
    }

    /// A node on a private DHT (the local testnet), relays off.
    fn dht_node(
        dir: &tempfile::TempDir,
        bootstrap: &[String],
        serves_blobs: bool,
        publish: bool,
    ) -> IrohNode {
        let options = NodeOptions {
            dht_lookup: true,
            dht_publish: publish,
            dht_bootstrap: Some(bootstrap.to_vec()),
            ..NodeOptions::default()
        };
        IrohNode::new_with_options(
            dir.path().to_path_buf(),
            false,
            None,
            serves_blobs,
            false,
            options,
        )
        .unwrap()
    }

    /// Node A publishes itself to a local DHT testnet; node B, which knows
    /// nothing but A's endpoint ID, finds A through the DHT and fetches from it.
    ///
    /// Offline-safe: the DHT is 3 local nodes on 127.0.0.1. Polls like
    /// iroh-mainline-address-lookup's own smoke test, because publishing is
    /// fire-and-forget.
    #[test]
    fn test_fetch_by_endpoint_id_alone_through_dht_testnet() {
        let dht_runtime = tokio::runtime::Runtime::new().unwrap();
        let testnet = dht_runtime
            .block_on(n0_mainline::Testnet::new(3))
            .expect("local DHT testnet");

        let (dir_a, dir_b, out) = (tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap());
        let server = dht_node(&dir_a, &testnet.bootstrap, true, true);
        let client = dht_node(&dir_b, &testnet.bootstrap, false, false);

        let data = test_bytes(200_000);
        let hash = server
            .put(&data)
            .unwrap()
            .parse::<BlobTicket>()
            .unwrap()
            .hash();

        let id: iroh::EndpointId = server.endpoint_id().parse().unwrap();
        let dest = out.path().join("via-dht.bin");
        let deadline = std::time::Instant::now() + Duration::from_secs(40);
        let result = loop {
            // No relay URL, no direct address: only address lookup can find A.
            let attempt = client.runtime().block_on(async {
                tokio::time::timeout(
                    Duration::from_secs(5),
                    fetch_to_path(
                        client.endpoint(),
                        hash,
                        EndpointAddr::from(id),
                        &dest,
                        u64::MAX,
                        &CancellationToken::new(),
                        |_, _| {},
                    ),
                )
                .await
            });
            match attempt {
                Ok(Ok(size)) => break Ok(size),
                other if std::time::Instant::now() > deadline => break Err(format!("{other:?}")),
                _ => std::thread::sleep(Duration::from_millis(500)),
            }
        };
        assert_eq!(result.expect("fetched through the DHT"), data.len() as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), data);

        client.shutdown().unwrap();
        server.shutdown().unwrap();
        drop(testnet);
    }

    #[test]
    fn test_dht_active_reports_whether_the_lookup_started() {
        let dht_runtime = tokio::runtime::Runtime::new().unwrap();
        let testnet = dht_runtime
            .block_on(n0_mainline::Testnet::new(3))
            .expect("local DHT testnet");

        let dir = tempdir().unwrap();
        let node = dht_node(&dir, &testnet.bootstrap, true, false);
        assert!(node.info().unwrap().dht_active);
        node.shutdown().unwrap();

        // No bootstrap node resolved (what an offline start sees): the node
        // starts without the DHT, does not hang, and says so.
        let dir = tempdir().unwrap();
        let start = std::time::Instant::now();
        let node = dht_node(&dir, &[], true, false);
        assert!(start.elapsed() < Duration::from_secs(5));
        assert!(!node.info().unwrap().dht_active);
        node.put(b"still works").unwrap();
        node.shutdown().unwrap();

        // Not requested: not active.
        let dir = tempdir().unwrap();
        let node = local_node(&dir, true);
        assert!(!node.info().unwrap().dht_active);
        node.shutdown().unwrap();
        drop(testnet);
    }

    /// Default bootstrap hostnames are resolved by us, bounded, never by the
    /// DHT crate's blocking DNS. With DoH dead and the OS resolver answering
    /// nothing for them, creation still returns within the bound.
    #[test]
    fn test_default_bootstrap_resolution_is_bounded() {
        use crate::resolve::HostResolverConfig;
        let dir = tempdir().unwrap();
        let start = std::time::Instant::now();
        let node = IrohNode::new_with_options(
            dir.path().to_path_buf(),
            false,
            None,
            false,
            false,
            NodeOptions {
                dht_lookup: true,
                host_resolver: HostResolverConfig {
                    // Nothing to ask over DoH, and a tiny OS budget.
                    doh_nameservers: Vec::new(),
                    system_timeout: Duration::from_millis(1),
                    ..HostResolverConfig::default()
                },
                ..NodeOptions::default()
            },
        )
        .unwrap();
        assert!(
            start.elapsed() < crate::resolve::DHT_BOOTSTRAP_RESOLVE_BOUND + Duration::from_secs(4)
        );
        node.shutdown().unwrap();
    }

    #[test]
    fn test_push_bytes_by_hostname_cancel_and_timeout() {
        let (dir_a, dir_b) = (tempdir().unwrap(), tempdir().unwrap());
        let server = push_receiver(&dir_a);
        let client = node_with_dead_doh(&dir_b, false);
        let target = |host: &str| {
            RemoteTarget::parse(&server.endpoint_id(), None, &[host.to_string()]).unwrap()
        };
        let run =
            |target: RemoteTarget, data: &[u8], timeout_ms: u64, token: &CancellationToken| {
                let shutdown = client.shutdown_token();
                client.runtime().block_on(run_cancellable(
                    push_bytes(
                        client.endpoint(),
                        client.store(),
                        client.host_resolver(),
                        target,
                        data,
                        timeout_ms,
                    ),
                    token,
                    &shutdown,
                ))
            };

        // By hostname.
        let data = test_bytes(5_000);
        let by_host = format!("localhost:{}", server_port(&server));
        let hash = run(target(&by_host), &data, 0, &CancellationToken::new()).unwrap();
        assert_eq!(hash.to_hex().to_string(), blake3_hex(&data));
        assert!(server.runtime().block_on(server.store().has(hash)).unwrap());

        // Cancelled before it starts: nothing is imported or sent.
        let other = b"cancelled before sending".to_vec();
        let token = CancellationToken::new();
        token.cancel();
        let result = run(target(&by_host), &other, 0, &token);
        assert!(
            matches!(result, Err(OperationError::Cancelled)),
            "{result:?}"
        );
        let other_hash = Hash::from_bytes(*blake3::hash(&other).as_bytes());
        assert!(
            !server
                .runtime()
                .block_on(server.store().has(other_hash))
                .unwrap()
        );
        assert!(
            !client
                .runtime()
                .block_on(client.store().has(other_hash))
                .unwrap()
        );

        // A blackholed address (TEST-NET-1) for an unreachable peer runs into the time limit.
        // Use a fresh peer ID so iroh does not reuse the cached localhost path from above.
        let blackhole_id = iroh::SecretKey::from_bytes(&[123u8; 32])
            .public()
            .to_string();
        let blackhole_target =
            RemoteTarget::parse(&blackhole_id, None, &["192.0.2.1:9".to_string()]).unwrap();
        let result = run(blackhole_target, b"x", 400, &CancellationToken::new());
        assert!(
            matches!(result, Err(OperationError::TimedOut)),
            "{result:?}"
        );

        client.shutdown().unwrap();
        server.shutdown().unwrap();
    }

    /// DHT lookup on, and the only direct address is a hostname that does not
    /// resolve (a DNS outage): the fetch falls through to address lookup by
    /// endpoint ID and still finds the node on the DHT.
    #[test]
    fn test_unresolvable_host_falls_through_to_dht_lookup() {
        use crate::resolve::HostResolverConfig;
        use iroh::dns::NameserverConfig;

        let dht_runtime = tokio::runtime::Runtime::new().unwrap();
        let testnet = dht_runtime
            .block_on(n0_mainline::Testnet::new(3))
            .expect("local DHT testnet");

        let (dir_a, dir_b, out) = (tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap());
        let server = dht_node(&dir_a, &testnet.bootstrap, true, true);
        let client = IrohNode::new_with_options(
            dir_b.path().to_path_buf(),
            false,
            None,
            false,
            false,
            NodeOptions {
                dht_lookup: true,
                dht_bootstrap: Some(testnet.bootstrap.clone()),
                host_resolver: HostResolverConfig {
                    doh_nameservers: vec![
                        NameserverConfig::https("127.0.0.1".parse().unwrap())
                            .with_port(1)
                            .with_tls_server_name("doh.invalid"),
                    ],
                    doh_timeout: Duration::from_millis(500),
                    system_timeout: Duration::from_secs(3),
                    ..HostResolverConfig::default()
                },
                ..NodeOptions::default()
            },
        )
        .unwrap();

        let data = test_bytes(100_000);
        let hash = server
            .put(&data)
            .unwrap()
            .parse::<BlobTicket>()
            .unwrap()
            .hash();
        let server_id = server.endpoint_id();
        let dest = out.path().join("fallthrough.bin");
        let deadline = std::time::Instant::now() + Duration::from_secs(40);
        let result = loop {
            let attempt = client.runtime().block_on(async {
                tokio::time::timeout(Duration::from_secs(8), async {
                    let target = RemoteTarget::parse(
                        &server_id,
                        None,
                        &["no-such-host.invalid:11204".to_string()],
                    )?;
                    let addr = client.host_resolver().resolve_target(target).await?;
                    assert_eq!(addr.ip_addrs().count(), 0, "nothing resolved");

                    fetch_to_path(
                        client.endpoint(),
                        hash,
                        addr,
                        &dest,
                        u64::MAX,
                        &CancellationToken::new(),
                        |_, _| {},
                    )
                    .await
                })
                .await
            });
            match attempt {
                Ok(Ok(size)) => break Ok(size),
                other if std::time::Instant::now() > deadline => break Err(format!("{other:?}")),
                _ => std::thread::sleep(Duration::from_millis(500)),
            }
        };
        assert_eq!(result.expect("fetched through the DHT"), data.len() as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), data);

        client.shutdown().unwrap();
        server.shutdown().unwrap();
        drop(testnet);
    }

    /// Without `dht_lookup` the same ID-only fetch has no way to find a
    /// relay-less node on a private DHT and fails (the control for the test above).
    #[test]
    fn test_id_only_fetch_fails_without_dht_lookup() {
        let dht_runtime = tokio::runtime::Runtime::new().unwrap();
        let testnet = dht_runtime
            .block_on(n0_mainline::Testnet::new(3))
            .expect("local DHT testnet");

        let (dir_a, dir_b, out) = (tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap());
        let server = dht_node(&dir_a, &testnet.bootstrap, true, true);
        let client = local_node(&dir_b, false);

        let hash = server
            .put(b"hello")
            .unwrap()
            .parse::<BlobTicket>()
            .unwrap()
            .hash();
        let id: iroh::EndpointId = server.endpoint_id().parse().unwrap();
        std::thread::sleep(Duration::from_secs(2));
        let result = client.runtime().block_on(async {
            tokio::time::timeout(
                Duration::from_secs(4),
                fetch_to_path(
                    client.endpoint(),
                    hash,
                    EndpointAddr::from(id),
                    &out.path().join("nope.bin"),
                    u64::MAX,
                    &CancellationToken::new(),
                    |_, _| {},
                ),
            )
            .await
        });
        assert!(!matches!(result, Ok(Ok(_))), "{result:?}");

        client.shutdown().unwrap();
        server.shutdown().unwrap();
        drop(testnet);
    }

    #[test]
    fn test_dht_publish_requires_lookup_and_serving() {
        let dir = tempdir().unwrap();
        let build = |dht_lookup, dht_publish, serves_blobs| {
            IrohNode::new_with_options(
                dir.path().to_path_buf(),
                false,
                None,
                serves_blobs,
                false,
                NodeOptions {
                    dht_lookup,
                    dht_publish,
                    ..NodeOptions::default()
                },
            )
        };
        let err = build(false, true, true)
            .err()
            .expect("publish without lookup");
        assert!(err.to_string().contains("dht_publish requires dht_lookup"));
        let err = build(true, true, false).err().expect("client-only publish");
        assert!(
            err.to_string()
                .contains("dht_publish requires serves_blobs")
        );
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
        let node = IrohNode::new(dir.path().to_path_buf(), true, None, true, false).unwrap();

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
        let node = IrohNode::new(dir.path().to_path_buf(), false, None, true, false).unwrap();

        let data = b"Hello, Iroh!";
        let ticket = node.put(data).unwrap();

        assert!(!ticket.is_empty());
        assert!(ticket.starts_with("blob")); // BlobTicket format

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
        let node_a = IrohNode::new(dir_a.path().to_path_buf(), true, None, true, false).unwrap();
        let node_b = IrohNode::new(dir_b.path().to_path_buf(), true, None, true, false).unwrap();

        let data = b"Cross-node get test data";
        let ticket = node_a.put(data).unwrap();

        let fetched = node_b.get_with_timeout(&ticket, 60_000).unwrap();
        assert_eq!(fetched, data);

        node_a.shutdown().unwrap();
        node_b.shutdown().unwrap();
    }

    #[test]
    fn test_client_only_node_shutdown_flushes_store_and_reopens() {
        let dir = tempdir().unwrap();
        let data = b"persisted by client-only node";

        let node1 = local_node(&dir, false);
        let ticket = node1.put(data).unwrap();
        let parsed: BlobTicket = ticket.parse().unwrap();
        node1.shutdown().unwrap();

        // Reopen the exact same FsStore directory and verify the blob survived.
        let node2 = local_node(&dir, false);
        let has = node2
            .runtime()
            .block_on(node2.store().has(parsed.hash()))
            .unwrap();
        assert!(has, "client-only FsStore must flush writes on shutdown");
        let bytes = node2
            .runtime()
            .block_on(node2.store().get_bytes(parsed.hash()))
            .unwrap();
        assert_eq!(bytes.as_ref(), data);
        node2.shutdown().unwrap();
    }

    #[test]
    fn test_get_ticket_uses_direct_ip_and_relay_hints_via_memory_lookup() {
        let dir_server = tempdir().unwrap();
        let dir_client = tempdir().unwrap();

        // Both nodes have relays disabled: get MUST use the ticket's direct IP
        // hints populated into MemoryLookup.
        let server = local_node(&dir_server, true);
        let client = local_node(&dir_client, false);

        let payload = test_bytes(300 * 1024);
        let ticket = server.put(&payload).unwrap();

        let mut progress = Vec::new();
        let got = client
            .get_with_progress(&ticket, |d, t| progress.push((d, t)))
            .unwrap();
        assert_eq!(got, payload);
        assert!(
            progress.last().copied() == Some((payload.len() as u64, payload.len() as u64)),
            "expected final progress with known total, got {progress:?}"
        );

        // Also test direct `get` on a fresh client node.
        let dir_client2 = tempdir().unwrap();
        let client2 = local_node(&dir_client2, false);
        assert_eq!(client2.get(&ticket).unwrap(), payload);

        // Verify a ticket carrying a custom relay URL populates MemoryLookup.
        let custom_relay: RelayUrl = "https://relay.example.invalid".parse().unwrap();
        let remote_key = iroh::SecretKey::from_bytes(&[42u8; 32]);
        let relay_addr =
            EndpointAddr::from(remote_key.public()).with_relay_url(custom_relay.clone());
        let relay_ticket =
            BlobTicket::new(relay_addr, Hash::new(b"x"), iroh_blobs::BlobFormat::Raw);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let _ = client2.runtime().block_on(get_from_ticket(
            client2.endpoint(),
            client2.store(),
            client2.memory_lookup(),
            &relay_ticket.to_string(),
            u64::MAX,
            100,
            &cancel,
            |_, _| {},
        ));
        let stored = client2
            .memory_lookup()
            .get_endpoint_info(remote_key.public())
            .expect("ticket relay hint should be stored in MemoryLookup");
        assert!(
            stored.relay_urls().any(|u| *u == custom_relay),
            "expected custom relay URL in MemoryLookup"
        );

        client2.shutdown().unwrap();
        client.shutdown().unwrap();
        server.shutdown().unwrap();
    }

    #[test]
    fn test_get_from_ticket_enforces_max_size_before_download() {
        let dir_server = tempdir().unwrap();
        let dir_client = tempdir().unwrap();

        let server = local_node(&dir_server, true);
        let client = local_node(&dir_client, false);

        let payload = test_bytes(16 * 1024);
        let ticket = server.put(&payload).unwrap();
        let parsed: BlobTicket = ticket.parse().unwrap();
        let cancel = CancellationToken::new();

        let err = client
            .runtime()
            .block_on(get_from_ticket(
                client.endpoint(),
                client.store(),
                client.memory_lookup(),
                &ticket,
                1024,
                0,
                &cancel,
                |_, _| {},
            ))
            .unwrap_err();
        assert!(
            matches!(err, OperationError::SizeLimitExceeded(n) if n == payload.len() as u64),
            "expected SizeLimitExceeded({}), got {err:?}",
            payload.len()
        );
        // Oversized blob must not have been written to the client's FsStore.
        let has = client
            .runtime()
            .block_on(client.store().has(parsed.hash()))
            .unwrap();
        assert!(!has, "oversized blob must not be written to local FsStore");

        // Local-hit maxSize enforcement on the server itself.
        let err = server
            .runtime()
            .block_on(get_from_ticket(
                server.endpoint(),
                server.store(),
                server.memory_lookup(),
                &ticket,
                100,
                0,
                &cancel,
                |_, _| {},
            ))
            .unwrap_err();
        assert!(matches!(err, OperationError::SizeLimitExceeded(n) if n == payload.len() as u64));

        client.shutdown().unwrap();
        server.shutdown().unwrap();
    }

    #[test]
    fn test_push_short_timeout_excludes_post_transfer_grace_sleep() {
        let dir_a = tempdir().unwrap();
        let dir_b = tempdir().unwrap();

        let node_a = local_node(&dir_a, false);
        let node_b = push_receiver(&dir_b);
        let direct = direct_addrs(&node_b);
        let data = b"fast localhost push";

        // 250 ms timeout is shorter than the 500 ms post-transfer grace wait;
        // push must still succeed because the grace period runs outside timeout.
        let hash_hex = node_a
            .push_to_node_with_timeout(&node_b.endpoint_id(), None, data, &direct, 250)
            .expect("push should not time out during post-transfer grace period");
        assert_eq!(hash_hex, blake3_hex(data));

        let hash: Hash = hash_hex.parse().unwrap();
        assert!(node_b.runtime().block_on(node_b.store().has(hash)).unwrap());

        node_a.shutdown().unwrap();
        node_b.shutdown().unwrap();
    }

    #[test]
    fn test_legacy_push_to_node_resolves_before_storing_blob() {
        let dir = tempdir().unwrap();
        let node = node_with_dead_doh(&dir, false);
        let data = b"must not be stored when target fails to resolve";
        let expected_hash = Hash::new(data);
        let remote_id = iroh::SecretKey::from_bytes(&[99u8; 32])
            .public()
            .to_string();

        let err = node.push_to_node(
            &remote_id,
            None,
            data,
            &["unresolvable.invalid:11204".into()],
        );
        assert!(err.is_err());

        let has = node
            .runtime()
            .block_on(node.store().has(expected_hash))
            .unwrap();
        assert!(
            !has,
            "legacy push_to_node must not persist blob before target hostname resolves"
        );
        node.shutdown().unwrap();
    }

    #[test]
    fn test_node_create_cancellable_honours_pre_cancelled_token() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("never-created");
        let cancel = CancellationToken::new();
        cancel.cancel();

        let res = IrohNode::new_with_options_cancellable(
            sub.clone(),
            false,
            None,
            true,
            false,
            NodeOptions::default(),
            &cancel,
        );
        assert!(matches!(res, Err(OperationError::Cancelled)));
        assert!(!sub.exists());
    }
}
