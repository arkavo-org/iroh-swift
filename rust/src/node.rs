//! IrohNode implementation wrapping iroh/iroh-blobs.
//!
//! Provides a minimal interface for blob storage and retrieval.

use anyhow::{Context, Result};
use iroh::{Endpoint, RelayMode, protocol::Router};
use iroh_blobs::{ALPN, BlobsProtocol, store::fs::FsStore, ticket::BlobTicket};
use std::path::PathBuf;
use tokio::runtime::Runtime;

/// Minimal Iroh node for blob operations.
///
/// Each node owns its own Tokio runtime to avoid conflicts with Swift's
/// concurrency model. All async operations are executed via `block_on`.
pub struct IrohNode {
    runtime: Runtime,
    endpoint: Endpoint,
    store: FsStore,
    router: Router,
}

impl IrohNode {
    /// Create a new Iroh node with persistent storage.
    ///
    /// # Arguments
    /// * `storage_path` - Directory for the blob store (created if doesn't exist)
    /// * `relay_enabled` - Whether to use n0's public relay servers
    pub fn new(storage_path: PathBuf, relay_enabled: bool) -> Result<Self> {
        // Create dedicated runtime for this node
        let runtime = Runtime::new().context("Failed to create Tokio runtime")?;

        let (endpoint, store, router) = runtime.block_on(async {
            // Create or load the persistent store
            let store = FsStore::load(&storage_path)
                .await
                .context("Failed to load blob store")?;

            // Build endpoint with optional relay
            let mut builder = Endpoint::builder();
            if !relay_enabled {
                builder = builder.relay_mode(RelayMode::Disabled);
            }
            // n0 public relays are default when relay_enabled=true

            let endpoint = builder.bind().await.context("Failed to bind endpoint")?;

            // Wait for relay connection if enabled
            if relay_enabled {
                let _ = endpoint.online().await;
            }

            // Set up the blobs protocol handler
            let blobs = BlobsProtocol::new(&store, None);
            let router = Router::builder(endpoint.clone())
                .accept(ALPN, blobs)
                .spawn();

            Ok::<_, anyhow::Error>((endpoint, store, router))
        })?;

        Ok(Self {
            runtime,
            endpoint,
            store,
            router,
        })
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
        let node = IrohNode::new(dir.path().to_path_buf(), false).unwrap();

        let data = b"Hello, Iroh!";
        let ticket = node.put(data).unwrap();

        assert!(!ticket.is_empty());
        assert!(ticket.starts_with("blob")); // BlobTicket format

        node.shutdown().unwrap();
    }
}
