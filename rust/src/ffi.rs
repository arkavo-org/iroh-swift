//! C ABI exports for Swift interop.
//!
//! This module provides the FFI boundary between Rust and Swift.
//! All functions use callback-based async patterns to integrate with
//! Swift's concurrency model.

use crate::node::IrohNode;
use iroh_blobs::ticket::BlobTicket;
use iroh_blobs::{BlobFormat, Hash, HashAndFormat};
use iroh_docs::Author;
use iroh_docs::DocTicket;
use iroh_docs::api::Doc;
use iroh_docs::api::protocol::{AddrInfoOptions, ShareMode};
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::PathBuf;

// ============================================================================
// Types
// ============================================================================

/// Borrowed bytes from Swift (read-only view into Swift memory).
#[repr(C)]
pub struct IrohBytes {
    pub data: *const u8,
    pub len: usize,
}

/// Owned bytes returned to Swift (must be freed with `iroh_bytes_free`).
#[repr(C)]
pub struct IrohOwnedBytes {
    pub data: *mut u8,
    pub len: usize,
    pub capacity: usize,
}

/// Configuration for creating a node.
#[repr(C)]
pub struct IrohNodeConfig {
    /// Path to the blob store directory (required).
    pub storage_path: *const c_char,
    /// Whether to use relay servers (default: true).
    pub relay_enabled: bool,
    /// Custom relay URL (null to use n0's public relays).
    /// Must be a valid URL like "https://relay.example.com".
    pub custom_relay_url: *const c_char,
    /// Whether to enable the Docs engine (default: false).
    /// When enabled, the node can create, join, and sync documents.
    pub docs_enabled: bool,
    /// Whether to accept the iroh-blobs protocol (default: true).
    /// When false the node is client-only: it can fetch and push blobs, but
    /// peers cannot download blobs from it or push blobs to it.
    pub serves_blobs: bool,
    /// Whether peers may push blobs into this node's store (default: false).
    /// Only meaningful with `serves_blobs`; setting it without `serves_blobs`
    /// fails node creation. When false, a serving node refuses push requests
    /// and writes nothing; peers can still download blobs from it.
    pub accepts_pushes: bool,
    /// Whether to resolve endpoint IDs on the BitTorrent Mainline DHT, so a
    /// node can be fetched from or pushed to by ID alone (default: false).
    pub dht_lookup: bool,
    /// Whether to also publish this node's address to the DHT (default: false).
    /// Requires `dht_lookup` and `serves_blobs`; otherwise node creation fails.
    pub dht_publish: bool,
}

/// Options for put/get operations.
#[repr(C)]
pub struct IrohOperationOptions {
    /// Timeout in milliseconds (0 = no timeout).
    pub timeout_ms: u64,
}

/// Opaque handle to an Iroh node.
///
/// The actual IrohNode is stored in a Box, and this handle holds
/// a raw pointer to that Box.
#[repr(C)]
pub struct IrohNodeHandle {
    _private: [u8; 0],
}

// ============================================================================
// Author Types
// ============================================================================

/// Author secret key (32 bytes).
///
/// This is the private key material used for signing document entries.
/// Must be kept secure (e.g., in iOS Keychain).
#[repr(C)]
pub struct IrohAuthorSecret {
    pub bytes: [u8; 32],
}

/// Author public ID (32 bytes).
///
/// This is the public identifier derived from the secret key.
/// Safe to share and store openly.
#[repr(C)]
pub struct IrohAuthorId {
    pub bytes: [u8; 32],
}

// ============================================================================
// Document Types
// ============================================================================

/// Opaque handle to an Iroh document.
///
/// Documents are syncing key-value stores shared between peers.
/// The handle wraps a Doc from iroh-docs.
#[repr(C)]
pub struct IrohDocHandle {
    _private: [u8; 0],
}

/// Internal document wrapper for FFI safety.
struct DocWrapper {
    doc: Doc,
    node_handle: *const IrohNodeHandle,
}

// Safety: DocWrapper is Send+Sync because Doc is Send+Sync and we
// only use node_handle for reads through the node's runtime.
unsafe impl Send for DocWrapper {}
unsafe impl Sync for DocWrapper {}

/// A document entry (key-value pair with metadata).
#[repr(C)]
pub struct IrohDocEntry {
    /// Author ID who wrote this entry (32 bytes).
    pub author_id: IrohAuthorId,
    /// Key bytes (owned, must be freed).
    pub key: IrohOwnedBytes,
    /// Content hash as hex string (must be freed with `iroh_string_free`).
    pub content_hash: *mut c_char,
    /// Size of the content in bytes.
    pub content_size: u64,
    /// Timestamp when entry was created (microseconds since epoch).
    pub timestamp: u64,
}

/// Share mode for document tickets.
#[repr(C)]
pub enum IrohDocShareMode {
    /// Read-only access.
    Read = 0,
    /// Read and write access.
    Write = 1,
}

// ============================================================================
// Blob Types
// ============================================================================

/// Blob format for tickets and tags.
#[repr(C)]
pub enum IrohBlobFormat {
    /// Raw single blob.
    Raw = 0,
    /// Hash sequence (collection of blobs).
    HashSeq = 1,
}

// ============================================================================
// Subscription Types
// ============================================================================

/// Opaque handle to a document subscription.
///
/// Used to cancel an active subscription.
#[repr(C)]
pub struct IrohSubscriptionHandle {
    _private: [u8; 0],
}

/// Internal subscription wrapper for cancellation.
struct SubscriptionWrapper {
    cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

/// Document event types.
#[repr(C)]
pub enum IrohDocEventType {
    /// A local insertion.
    InsertLocal = 0,
    /// Received a remote insert.
    InsertRemote = 1,
    /// Content is now available locally.
    ContentReady = 2,
    /// All pending content is ready.
    PendingContentReady = 3,
    /// A new neighbor joined the swarm.
    NeighborUp = 4,
    /// A neighbor left the swarm.
    NeighborDown = 5,
    /// Sync finished with a peer.
    SyncFinished = 6,
}

/// A document event from subscription.
#[repr(C)]
pub struct IrohDocEvent {
    /// The type of event.
    pub event_type: IrohDocEventType,
    /// The entry for insert events (null for other events).
    /// Must be freed with `iroh_doc_entry_free` if not null.
    pub entry: *const IrohDocEntry,
    /// The peer ID for remote events (null for local events).
    /// Must be freed with `iroh_string_free` if not null.
    pub peer_id: *const c_char,
    /// The content hash for ContentReady events (null for other events).
    /// Must be freed with `iroh_string_free` if not null.
    pub content_hash: *const c_char,
}

// ============================================================================
// Callbacks
// ============================================================================

/// Callback for operations that return a string on success.
///
/// NOTE: Callbacks run on a Tokio worker thread. Swift must safely
/// resume continuations from that context.
#[repr(C)]
pub struct IrohCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called on success with a C string (caller must free with `iroh_string_free`).
    pub on_success: extern "C" fn(userdata: *mut c_void, result: *const c_char),
    /// Called on failure with an error message (caller must free with `iroh_string_free`).
    pub on_failure: extern "C" fn(userdata: *mut c_void, error: *const c_char),
}

/// Callback for operations that return bytes on success.
#[repr(C)]
pub struct IrohGetCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called on success with owned bytes (caller must free with `iroh_bytes_free`).
    pub on_success: extern "C" fn(userdata: *mut c_void, bytes: IrohOwnedBytes),
    /// Called on failure with an error message (caller must free with `iroh_string_free`).
    pub on_failure: extern "C" fn(userdata: *mut c_void, error: *const c_char),
}

/// Callback for node creation.
#[repr(C)]
pub struct IrohNodeCreateCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called on success with the node handle.
    pub on_success: extern "C" fn(userdata: *mut c_void, handle: *mut IrohNodeHandle),
    /// Called on failure with an error message (caller must free with `iroh_string_free`).
    pub on_failure: extern "C" fn(userdata: *mut c_void, error: *const c_char),
}

/// Progress information for a download operation.
#[repr(C)]
pub struct IrohDownloadProgress {
    /// Bytes downloaded so far.
    pub downloaded: u64,
    /// Total bytes expected (0 if unknown).
    pub total: u64,
}

/// Callback for get operations with progress reporting.
#[repr(C)]
pub struct IrohGetProgressCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called with progress updates during download.
    pub on_progress: extern "C" fn(userdata: *mut c_void, progress: IrohDownloadProgress),
    /// Called on success with owned bytes (caller must free with `iroh_bytes_free`).
    pub on_success: extern "C" fn(userdata: *mut c_void, bytes: IrohOwnedBytes),
    /// Called on failure with an error message (caller must free with `iroh_string_free`).
    pub on_failure: extern "C" fn(userdata: *mut c_void, error: *const c_char),
}

/// Information about an Iroh node.
#[repr(C)]
pub struct IrohNodeInfo {
    /// Node ID as a string (caller must free with `iroh_string_free`).
    pub node_id: *const c_char,
    /// Relay URL if connected (caller must free with `iroh_string_free`).
    /// Null if not connected to a relay.
    pub relay_url: *const c_char,
    /// Whether the node is connected to the network.
    pub is_connected: bool,
    /// Direct `ip:port` addresses, separated by `\n` (caller must free with
    /// `iroh_string_free`). Null if the node has no direct addresses.
    pub direct_addrs: *const c_char,
    /// Whether the Mainline DHT address lookup is running (false if not
    /// requested, or if no bootstrap node resolved at creation).
    pub dht_active: bool,
}

/// Callback for node info retrieval.
#[repr(C)]
pub struct IrohNodeInfoCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called on success with node info.
    pub on_success: extern "C" fn(userdata: *mut c_void, info: IrohNodeInfo),
    /// Called on failure with an error message (caller must free with `iroh_string_free`).
    pub on_failure: extern "C" fn(userdata: *mut c_void, error: *const c_char),
}

/// Parsed ticket information.
#[repr(C)]
pub struct IrohTicketInfo {
    /// Whether the ticket is valid.
    pub is_valid: bool,
    /// The blob hash as a string (caller must free with `iroh_string_free`).
    /// Null if invalid.
    pub hash: *const c_char,
    /// The node ID from the ticket (caller must free with `iroh_string_free`).
    /// Null if invalid.
    pub node_id: *const c_char,
    /// Whether this is a recursive (collection) ticket.
    pub is_recursive: bool,
}

/// Callback for ticket validation.
#[repr(C)]
pub struct IrohTicketValidateCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called with validation result. Always called (never fails).
    pub on_complete: extern "C" fn(userdata: *mut c_void, info: IrohTicketInfo),
}

/// Callback for node close operation.
#[repr(C)]
pub struct IrohCloseCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called when close completes successfully.
    pub on_complete: extern "C" fn(userdata: *mut c_void),
    /// Called if close fails with an error message.
    pub on_failure: extern "C" fn(userdata: *mut c_void, error: *const c_char),
}

// Safety: The userdata pointer is exclusively owned by the callback and only
// accessed on the thread that calls on_complete/on_failure. The function
// pointers are plain C functions safe to call from any thread.
unsafe impl Send for IrohCloseCallback {}

/// Callback for author creation.
#[repr(C)]
pub struct IrohAuthorCreateCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called on success with the author secret and ID.
    pub on_success:
        extern "C" fn(userdata: *mut c_void, secret: IrohAuthorSecret, id: IrohAuthorId),
    /// Called on failure with an error message (caller must free with `iroh_string_free`).
    pub on_failure: extern "C" fn(userdata: *mut c_void, error: *const c_char),
}

/// Callback for document creation/join operations.
#[repr(C)]
pub struct IrohDocCreateCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called on success with the document handle and namespace ID.
    pub on_success: extern "C" fn(
        userdata: *mut c_void,
        handle: *mut IrohDocHandle,
        namespace_id: *const c_char,
    ),
    /// Called on failure with an error message (caller must free with `iroh_string_free`).
    pub on_failure: extern "C" fn(userdata: *mut c_void, error: *const c_char),
}

/// Callback for document get operations.
#[repr(C)]
pub struct IrohDocGetCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called on success with the entry (may be null if not found).
    /// Caller must free entry with `iroh_doc_entry_free` if not null.
    pub on_success: extern "C" fn(userdata: *mut c_void, entry: *const IrohDocEntry),
    /// Called on failure with an error message.
    pub on_failure: extern "C" fn(userdata: *mut c_void, error: *const c_char),
}

/// Callback for document set operations.
#[repr(C)]
pub struct IrohDocSetCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called on success with the content hash (caller must free with `iroh_string_free`).
    pub on_success: extern "C" fn(userdata: *mut c_void, hash: *const c_char),
    /// Called on failure with an error message.
    pub on_failure: extern "C" fn(userdata: *mut c_void, error: *const c_char),
}

/// Callback for document delete operations.
#[repr(C)]
pub struct IrohDocDelCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called on success with count of deleted entries.
    pub on_success: extern "C" fn(userdata: *mut c_void, deleted_count: u64),
    /// Called on failure with an error message.
    pub on_failure: extern "C" fn(userdata: *mut c_void, error: *const c_char),
}

/// Streaming callback for get_many (prefix queries).
/// Called multiple times - once per entry, then on_complete.
#[repr(C)]
pub struct IrohDocGetManyCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called for each entry found. Entry must be freed with `iroh_doc_entry_free`.
    pub on_entry: extern "C" fn(userdata: *mut c_void, entry: *const IrohDocEntry),
    /// Called when iteration completes successfully.
    pub on_complete: extern "C" fn(userdata: *mut c_void),
    /// Called on error. No more callbacks after this.
    pub on_failure: extern "C" fn(userdata: *mut c_void, error: *const c_char),
}

/// Streaming callback for document subscriptions.
/// Called multiple times - once per event, then on_complete when stream ends.
#[repr(C)]
pub struct IrohDocSubscribeCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called for each event. Event must be freed with `iroh_doc_event_free`.
    pub on_event: extern "C" fn(userdata: *mut c_void, event: IrohDocEvent),
    /// Called when subscription ends normally.
    pub on_complete: extern "C" fn(userdata: *mut c_void),
    /// Called on error. No more callbacks after this.
    pub on_failure: extern "C" fn(userdata: *mut c_void, error: *const c_char),
}

// ============================================================================
// Node Lifecycle
// ============================================================================

/// Create a new Iroh node asynchronously.
///
/// # Safety
/// - `config.storage_path` must be a valid null-terminated UTF-8 string
/// - `config.custom_relay_url` must be null or a valid null-terminated UTF-8 string
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub extern "C" fn iroh_node_create(config: IrohNodeConfig, callback: IrohNodeCreateCallback) {
    // Parse the storage path
    let storage_path = if config.storage_path.is_null() {
        let error = CString::new("storage_path cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    } else {
        let path_str = unsafe { CStr::from_ptr(config.storage_path) };
        match path_str.to_str() {
            Ok(s) => PathBuf::from(s),
            Err(e) => {
                let error = CString::new(format!("Invalid storage path: {}", e)).unwrap();
                (callback.on_failure)(callback.userdata, error.into_raw());
                return;
            }
        }
    };

    // Parse optional custom relay URL
    let custom_relay_url = if config.custom_relay_url.is_null() {
        None
    } else {
        let url_str = unsafe { CStr::from_ptr(config.custom_relay_url) };
        match url_str.to_str() {
            Ok(s) => Some(s.to_string()),
            Err(e) => {
                let error = CString::new(format!("Invalid custom relay URL: {}", e)).unwrap();
                (callback.on_failure)(callback.userdata, error.into_raw());
                return;
            }
        }
    };

    let relay_enabled = config.relay_enabled;
    let docs_enabled = config.docs_enabled;
    let serves_blobs = config.serves_blobs;
    let accepts_pushes = config.accepts_pushes;
    let options = crate::node::NodeOptions {
        dht_lookup: config.dht_lookup,
        dht_publish: config.dht_publish,
        ..Default::default()
    };

    // Create the node synchronously
    // Note: Swift should call this from a background thread/task
    match IrohNode::new_with_options(
        storage_path,
        relay_enabled,
        custom_relay_url,
        docs_enabled,
        serves_blobs,
        accepts_pushes,
        options,
    ) {
        Ok(node) => {
            // Box the node and convert to raw pointer
            let boxed = Box::new(node);
            let handle = Box::into_raw(boxed) as *mut IrohNodeHandle;
            (callback.on_success)(callback.userdata, handle);
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

/// Destroy an Iroh node and free its resources.
///
/// Performs shutdown on a background thread to avoid blocking Swift's
/// cooperative thread pool (which can deadlock if called from deinit).
///
/// # Safety
/// - `handle` must be a valid pointer returned by `iroh_node_create`
/// - `handle` must not be used after this call
#[unsafe(no_mangle)]
pub extern "C" fn iroh_node_destroy(handle: *mut IrohNodeHandle) {
    if handle.is_null() {
        return;
    }

    unsafe {
        let node = Box::from_raw(handle as *mut IrohNode);
        // Shutdown on a dedicated thread so we never block Swift's
        // cooperative thread pool. The thread is detached — if the
        // process is exiting, the OS cleans up.
        std::thread::spawn(move || {
            let _ = node.shutdown();
        });
    }
}

// ============================================================================
// Core Operations
// ============================================================================

/// Add bytes to the blob store and get a shareable ticket.
///
/// # Safety
/// - `handle` must be a valid node handle
/// - `bytes.data` must point to valid memory for `bytes.len` bytes
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub extern "C" fn iroh_put(
    handle: *const IrohNodeHandle,
    bytes: IrohBytes,
    callback: IrohCallback,
) {
    if handle.is_null() {
        let error = CString::new("handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    // Copy the bytes to own them (Swift memory may not be stable)
    let data = if bytes.data.is_null() || bytes.len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(bytes.data, bytes.len).to_vec() }
    };

    // Get reference to node (we don't own it)
    let node = unsafe { &*(handle as *const IrohNode) };

    // Perform the put operation
    // Note: This blocks on the node's runtime, which is intentional
    match node.put(&data) {
        Ok(ticket) => {
            let ticket_cstr = CString::new(ticket).unwrap();
            (callback.on_success)(callback.userdata, ticket_cstr.into_raw());
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

/// Download bytes from a ticket.
///
/// # Safety
/// - `handle` must be a valid node handle
/// - `ticket` must be a valid null-terminated UTF-8 string
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_get(
    handle: *const IrohNodeHandle,
    ticket: *const c_char,
    callback: IrohGetCallback,
) {
    if handle.is_null() {
        let error = CString::new("handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    if ticket.is_null() {
        let error = CString::new("ticket cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    // Parse the ticket string
    let ticket_str = match unsafe { CStr::from_ptr(ticket) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = CString::new(format!("Invalid ticket string: {}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    // Get reference to node
    let node = unsafe { &*(handle as *const IrohNode) };

    // Perform the get operation
    match node.get(&ticket_str) {
        Ok(bytes) => {
            let mut vec = bytes;
            let owned = IrohOwnedBytes {
                data: vec.as_mut_ptr(),
                len: vec.len(),
                capacity: vec.capacity(),
            };
            std::mem::forget(vec); // Prevent deallocation, Swift will free
            (callback.on_success)(callback.userdata, owned);
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

// ============================================================================
// Memory Management
// ============================================================================

/// Free a string returned by Iroh functions.
///
/// # Safety
/// - `s` must be a pointer returned by an Iroh function, or null
/// - `s` must not be used after this call
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_string_free(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            drop(CString::from_raw(s));
        }
    }
}

/// Free bytes returned by `iroh_get`.
///
/// # Safety
/// - `bytes` must have been returned by `iroh_get`
/// - The bytes must not be used after this call
#[unsafe(no_mangle)]
pub extern "C" fn iroh_bytes_free(bytes: IrohOwnedBytes) {
    if !bytes.data.is_null() {
        unsafe {
            // Reconstruct the Vec and let it drop
            drop(Vec::from_raw_parts(bytes.data, bytes.len, bytes.capacity));
        }
    }
}

// ============================================================================
// Extended Operations
// ============================================================================

/// Download bytes from a ticket with progress reporting.
///
/// # Safety
/// - `handle` must be a valid node handle
/// - `ticket` must be a valid null-terminated UTF-8 string
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_get_with_progress(
    handle: *const IrohNodeHandle,
    ticket: *const c_char,
    callback: IrohGetProgressCallback,
) {
    if handle.is_null() {
        let error = CString::new("handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    if ticket.is_null() {
        let error = CString::new("ticket cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    // Parse the ticket string
    let ticket_str = match unsafe { CStr::from_ptr(ticket) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = CString::new(format!("Invalid ticket string: {}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    let node = unsafe { &*(handle as *const IrohNode) };
    let userdata = callback.userdata;
    let on_progress_fn = callback.on_progress;

    // Progress callback closure
    let progress_fn = move |downloaded: u64, total: u64| {
        let progress = IrohDownloadProgress { downloaded, total };
        (on_progress_fn)(userdata, progress);
    };

    match node.get_with_progress(&ticket_str, progress_fn) {
        Ok(bytes) => {
            let mut vec = bytes;
            let owned = IrohOwnedBytes {
                data: vec.as_mut_ptr(),
                len: vec.len(),
                capacity: vec.capacity(),
            };
            std::mem::forget(vec);
            (callback.on_success)(callback.userdata, owned);
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

/// Get information about the node.
///
/// # Safety
/// - `handle` must be a valid node handle
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub extern "C" fn iroh_node_info(handle: *const IrohNodeHandle, callback: IrohNodeInfoCallback) {
    if handle.is_null() {
        let error = CString::new("handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    let node = unsafe { &*(handle as *const IrohNode) };

    match node.info() {
        Ok(info) => {
            let node_id = CString::new(info.node_id).unwrap().into_raw();
            let relay_url = info
                .relay_url
                .map(|url| CString::new(url).unwrap().into_raw())
                .unwrap_or(std::ptr::null_mut());

            let direct_addrs = if info.direct_addrs.is_empty() {
                std::ptr::null_mut()
            } else {
                CString::new(info.direct_addrs.join("\n"))
                    .unwrap()
                    .into_raw()
            };

            let ffi_info = IrohNodeInfo {
                node_id,
                relay_url,
                is_connected: info.is_connected,
                direct_addrs,
                dht_active: info.dht_active,
            };
            (callback.on_success)(callback.userdata, ffi_info);
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

/// Validate and parse a ticket string.
///
/// This function always succeeds - check `info.is_valid` for the result.
///
/// # Safety
/// - `ticket` must be a valid null-terminated UTF-8 string (or null)
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_validate_ticket(
    ticket: *const c_char,
    callback: IrohTicketValidateCallback,
) {
    let result = if ticket.is_null() {
        IrohTicketInfo {
            is_valid: false,
            hash: std::ptr::null(),
            node_id: std::ptr::null(),
            is_recursive: false,
        }
    } else {
        match unsafe { CStr::from_ptr(ticket) }.to_str() {
            Ok(ticket_str) => match ticket_str.parse::<BlobTicket>() {
                Ok(parsed) => {
                    let hash = CString::new(parsed.hash().to_string()).unwrap().into_raw();
                    let node_id = CString::new(parsed.addr().id.to_string())
                        .unwrap()
                        .into_raw();

                    IrohTicketInfo {
                        is_valid: true,
                        hash,
                        node_id,
                        is_recursive: parsed.recursive(),
                    }
                }
                Err(_) => IrohTicketInfo {
                    is_valid: false,
                    hash: std::ptr::null(),
                    node_id: std::ptr::null(),
                    is_recursive: false,
                },
            },
            Err(_) => IrohTicketInfo {
                is_valid: false,
                hash: std::ptr::null(),
                node_id: std::ptr::null(),
                is_recursive: false,
            },
        }
    };

    (callback.on_complete)(callback.userdata, result);
}

// ============================================================================
// Close and Timeout Operations
// ============================================================================

/// Explicitly close a node and free its resources asynchronously.
///
/// This is preferred over `iroh_node_destroy` when you need to await
/// graceful shutdown completion.
///
/// # Safety
/// - `handle` must be a valid pointer returned by `iroh_node_create`
/// - `handle` must not be used after this call
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub extern "C" fn iroh_node_close(handle: *mut IrohNodeHandle, callback: IrohCloseCallback) {
    if handle.is_null() {
        (callback.on_complete)(callback.userdata);
        return;
    }

    // Run shutdown on a dedicated thread to avoid blocking Swift's
    // cooperative thread pool, which can cause deadlocks.
    let node = unsafe { Box::from_raw(handle as *mut IrohNode) };
    let on_complete = callback.on_complete;
    let on_failure = callback.on_failure;
    let userdata = callback.userdata as usize;
    std::thread::spawn(move || {
        let userdata = userdata as *mut c_void;
        match node.shutdown() {
            Ok(()) => (on_complete)(userdata),
            Err(e) => {
                let error = CString::new(format!("{:#}", e)).unwrap();
                (on_failure)(userdata, error.into_raw());
            }
        }
    });
}

/// Add bytes to the blob store with options (e.g., timeout).
///
/// # Safety
/// - `handle` must be a valid node handle
/// - `bytes.data` must point to valid memory for `bytes.len` bytes
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub extern "C" fn iroh_put_with_options(
    handle: *const IrohNodeHandle,
    bytes: IrohBytes,
    options: IrohOperationOptions,
    callback: IrohCallback,
) {
    if handle.is_null() {
        let error = CString::new("handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    // Copy the bytes to own them (Swift memory may not be stable)
    let data = if bytes.data.is_null() || bytes.len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(bytes.data, bytes.len).to_vec() }
    };

    let node = unsafe { &*(handle as *const IrohNode) };
    let timeout_ms = options.timeout_ms;

    match node.put_with_timeout(&data, timeout_ms) {
        Ok(ticket) => {
            let ticket_cstr = CString::new(ticket).unwrap();
            (callback.on_success)(callback.userdata, ticket_cstr.into_raw());
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

/// Push bytes to a remote node identified by its node ID.
///
/// Blocks the calling thread and cannot be cancelled (hostname resolution
/// included); `iroh_push_bytes_to_node` is the cancellable form Swift uses.
///
/// Adds the data to the local store, connects to the remote node,
/// and pushes the blob. Returns the blob hash as a hex string on success.
///
/// The remote must accept pushes (`accepts_pushes` on an iroh-swift node). A
/// node that refuses them stores nothing; the pusher is not reliably told (a
/// small blob can still report success, a larger one fails).
///
/// # Safety
/// - `handle` must be a valid node handle
/// - `remote_node_id_hex` must be a valid null-terminated UTF-8 string
/// - `relay_url` is nullable; if non-null must be a valid null-terminated UTF-8 string
/// - `direct_addrs` is nullable; if non-null must point to an array of `direct_addrs_len` valid
///   null-terminated UTF-8 strings
/// - `bytes.data` must point to valid memory for `bytes.len` bytes
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_push_to_node(
    handle: *const IrohNodeHandle,
    remote_node_id_hex: *const c_char,
    relay_url: *const c_char,
    direct_addrs: *const *const c_char,
    direct_addrs_len: usize,
    bytes: IrohBytes,
    callback: IrohCallback,
) {
    if handle.is_null() {
        let error = CString::new("handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    if remote_node_id_hex.is_null() {
        let error = CString::new("remote_node_id_hex cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    // Parse remote node ID hex string
    let node_id_str = match unsafe { CStr::from_ptr(remote_node_id_hex) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = CString::new(format!("Invalid remote_node_id_hex string: {}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    // Parse optional relay URL
    let relay_url_str = if relay_url.is_null() {
        None
    } else {
        match unsafe { CStr::from_ptr(relay_url) }.to_str() {
            Ok(s) => Some(s.to_string()),
            Err(e) => {
                let error = CString::new(format!("Invalid relay_url string: {}", e)).unwrap();
                (callback.on_failure)(callback.userdata, error.into_raw());
                return;
            }
        }
    };

    // Parse optional direct addresses array
    let addrs = if direct_addrs.is_null() || direct_addrs_len == 0 {
        Vec::new()
    } else {
        let slice = unsafe { std::slice::from_raw_parts(direct_addrs, direct_addrs_len) };
        let mut result = Vec::with_capacity(direct_addrs_len);
        for &ptr in slice {
            if ptr.is_null() {
                continue;
            }
            match unsafe { CStr::from_ptr(ptr) }.to_str() {
                Ok(s) => result.push(s.to_string()),
                Err(e) => {
                    let error =
                        CString::new(format!("Invalid direct address string: {}", e)).unwrap();
                    (callback.on_failure)(callback.userdata, error.into_raw());
                    return;
                }
            }
        }
        result
    };

    // Copy the bytes to own them (Swift memory may not be stable)
    let data = if bytes.data.is_null() || bytes.len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(bytes.data, bytes.len).to_vec() }
    };

    let node = unsafe { &*(handle as *const IrohNode) };

    match node.push_to_node(&node_id_str, relay_url_str.as_deref(), &data, &addrs) {
        Ok(hash_hex) => {
            let result = CString::new(hash_hex).unwrap();
            (callback.on_success)(callback.userdata, result.into_raw());
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

/// Push bytes to a remote node with options (e.g., timeout).
///
/// Same as `iroh_push_to_node` but with configurable operation options. Also
/// blocking and not cancellable; see `iroh_push_bytes_to_node`.
///
/// # Safety
/// - `handle` must be a valid node handle
/// - `remote_node_id_hex` must be a valid null-terminated UTF-8 string
/// - `relay_url` is nullable; if non-null must be a valid null-terminated UTF-8 string
/// - `direct_addrs` is nullable; if non-null must point to an array of `direct_addrs_len` valid
///   null-terminated UTF-8 strings
/// - `bytes.data` must point to valid memory for `bytes.len` bytes
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_push_to_node_with_options(
    handle: *const IrohNodeHandle,
    remote_node_id_hex: *const c_char,
    relay_url: *const c_char,
    direct_addrs: *const *const c_char,
    direct_addrs_len: usize,
    bytes: IrohBytes,
    options: IrohOperationOptions,
    callback: IrohCallback,
) {
    if handle.is_null() {
        let error = CString::new("handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    if remote_node_id_hex.is_null() {
        let error = CString::new("remote_node_id_hex cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    // Parse remote node ID hex string
    let node_id_str = match unsafe { CStr::from_ptr(remote_node_id_hex) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = CString::new(format!("Invalid remote_node_id_hex string: {}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    // Parse optional relay URL
    let relay_url_str = if relay_url.is_null() {
        None
    } else {
        match unsafe { CStr::from_ptr(relay_url) }.to_str() {
            Ok(s) => Some(s.to_string()),
            Err(e) => {
                let error = CString::new(format!("Invalid relay_url string: {}", e)).unwrap();
                (callback.on_failure)(callback.userdata, error.into_raw());
                return;
            }
        }
    };

    // Parse optional direct addresses array
    let addrs = if direct_addrs.is_null() || direct_addrs_len == 0 {
        Vec::new()
    } else {
        let slice = unsafe { std::slice::from_raw_parts(direct_addrs, direct_addrs_len) };
        let mut result = Vec::with_capacity(direct_addrs_len);
        for &ptr in slice {
            if ptr.is_null() {
                continue;
            }
            match unsafe { CStr::from_ptr(ptr) }.to_str() {
                Ok(s) => result.push(s.to_string()),
                Err(e) => {
                    let error =
                        CString::new(format!("Invalid direct address string: {}", e)).unwrap();
                    (callback.on_failure)(callback.userdata, error.into_raw());
                    return;
                }
            }
        }
        result
    };

    // Copy the bytes to own them (Swift memory may not be stable)
    let data = if bytes.data.is_null() || bytes.len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(bytes.data, bytes.len).to_vec() }
    };

    let node = unsafe { &*(handle as *const IrohNode) };
    let timeout_ms = options.timeout_ms;

    match node.push_to_node_with_timeout(
        &node_id_str,
        relay_url_str.as_deref(),
        &data,
        &addrs,
        timeout_ms,
    ) {
        Ok(hash_hex) => {
            let result = CString::new(hash_hex).unwrap();
            (callback.on_success)(callback.userdata, result.into_raw());
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

/// Download bytes from a ticket with options (e.g., timeout).
///
/// # Safety
/// - `handle` must be a valid node handle
/// - `ticket` must be a valid null-terminated UTF-8 string
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_get_with_options(
    handle: *const IrohNodeHandle,
    ticket: *const c_char,
    options: IrohOperationOptions,
    callback: IrohGetCallback,
) {
    if handle.is_null() {
        let error = CString::new("handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    if ticket.is_null() {
        let error = CString::new("ticket cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    let ticket_str = match unsafe { CStr::from_ptr(ticket) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = CString::new(format!("Invalid ticket string: {}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    let node = unsafe { &*(handle as *const IrohNode) };
    let timeout_ms = options.timeout_ms;

    match node.get_with_timeout(&ticket_str, timeout_ms) {
        Ok(bytes) => {
            let mut vec = bytes;
            let owned = IrohOwnedBytes {
                data: vec.as_mut_ptr(),
                len: vec.len(),
                capacity: vec.capacity(),
            };
            std::mem::forget(vec);
            (callback.on_success)(callback.userdata, owned);
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

// ============================================================================
// Author Operations
// ============================================================================

/// Create a new random author keypair.
///
/// The secret key should be stored securely (e.g., in iOS Keychain).
/// The ID is derived from the secret and can be stored openly.
///
/// # Safety
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub extern "C" fn iroh_author_create(callback: IrohAuthorCreateCallback) {
    // Generate a new random author
    let author = Author::new(&mut rand::rng());

    // Get the secret bytes (32 bytes)
    let secret_bytes = author.to_bytes();
    let secret = IrohAuthorSecret {
        bytes: secret_bytes,
    };

    // Get the public ID bytes (32 bytes)
    let author_id = author.id();
    let id_bytes = author_id.as_bytes();
    let id = IrohAuthorId { bytes: *id_bytes };

    (callback.on_success)(callback.userdata, secret, id);
}

/// Get the author ID from a secret key.
///
/// This is a pure computation - no node required.
/// Useful for deriving the ID after loading secret from Keychain.
///
/// # Safety
/// - `secret` must contain valid author secret bytes
#[unsafe(no_mangle)]
pub extern "C" fn iroh_author_id_from_secret(secret: IrohAuthorSecret) -> IrohAuthorId {
    // Reconstruct the Author from the secret bytes
    let author = Author::from_bytes(&secret.bytes);

    // Get the public ID bytes
    let author_id = author.id();
    let id_bytes = author_id.as_bytes();
    IrohAuthorId { bytes: *id_bytes }
}

/// Import an author from a hex-encoded secret key.
///
/// Useful for debugging or cross-device sync.
///
/// # Safety
/// - `secret_hex` must be a valid null-terminated UTF-8 string containing 64 hex chars
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_author_from_hex(
    secret_hex: *const c_char,
    callback: IrohAuthorCreateCallback,
) {
    if secret_hex.is_null() {
        let error = CString::new("secret_hex cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    let hex_str = match unsafe { CStr::from_ptr(secret_hex) }.to_str() {
        Ok(s) => s,
        Err(e) => {
            let error = CString::new(format!("Invalid UTF-8 in secret_hex: {}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    // Decode hex to bytes
    let secret_bytes: [u8; 32] = match hex::decode(hex_str) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            arr
        }
        Ok(bytes) => {
            let error = CString::new(format!(
                "Invalid secret length: expected 32 bytes, got {}",
                bytes.len()
            ))
            .unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
        Err(e) => {
            let error = CString::new(format!("Invalid hex string: {}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    // Reconstruct the Author
    let author = Author::from_bytes(&secret_bytes);

    let secret = IrohAuthorSecret {
        bytes: secret_bytes,
    };
    let id = IrohAuthorId {
        bytes: *author.id().as_bytes(),
    };

    (callback.on_success)(callback.userdata, secret, id);
}

/// Export an author secret as a hex string.
///
/// Useful for debugging or backup.
///
/// # Safety
/// - The returned string must be freed with `iroh_string_free`
#[unsafe(no_mangle)]
pub extern "C" fn iroh_author_secret_to_hex(secret: IrohAuthorSecret) -> *mut c_char {
    let hex_string = hex::encode(secret.bytes);
    CString::new(hex_string).unwrap().into_raw()
}

/// Export an author ID as a hex string.
///
/// # Safety
/// - The returned string must be freed with `iroh_string_free`
#[unsafe(no_mangle)]
pub extern "C" fn iroh_author_id_to_hex(id: IrohAuthorId) -> *mut c_char {
    let hex_string = hex::encode(id.bytes);
    CString::new(hex_string).unwrap().into_raw()
}

/// Import an author into the docs engine.
///
/// This must be called before using an author to sign document entries.
/// The author is registered with the docs engine so it can sign entries.
///
/// # Safety
/// - `handle` must be a valid node handle with docs enabled
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub extern "C" fn iroh_author_import(
    handle: *const IrohNodeHandle,
    author_secret: IrohAuthorSecret,
    callback: IrohCloseCallback,
) {
    if handle.is_null() {
        let error = CString::new("handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    let node = unsafe { &*(handle as *const IrohNode) };

    let docs = match node.docs() {
        Some(d) => d,
        None => {
            let error = CString::new("docs not enabled on this node").unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    // Reconstruct the author from secret bytes
    let author = Author::from_bytes(&author_secret.bytes);

    match node.runtime().block_on(docs.api().author_import(author)) {
        Ok(()) => {
            (callback.on_complete)(callback.userdata);
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

// ============================================================================
// Document Operations
// ============================================================================

/// Create a new document.
///
/// # Safety
/// - `handle` must be a valid node handle with docs enabled
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub extern "C" fn iroh_doc_create(handle: *const IrohNodeHandle, callback: IrohDocCreateCallback) {
    if handle.is_null() {
        let error = CString::new("handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    let node = unsafe { &*(handle as *const IrohNode) };

    let docs = match node.docs() {
        Some(d) => d,
        None => {
            let error = CString::new("docs not enabled on this node").unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    match node.runtime().block_on(docs.api().create()) {
        Ok(doc) => {
            let namespace_id = doc.id().to_string();
            let namespace_cstr = CString::new(namespace_id).unwrap().into_raw();

            // Wrap the doc for FFI
            let wrapper = Box::new(DocWrapper {
                doc,
                node_handle: handle,
            });
            let doc_handle = Box::into_raw(wrapper) as *mut IrohDocHandle;

            (callback.on_success)(callback.userdata, doc_handle, namespace_cstr);
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

/// Join an existing document via ticket.
///
/// # Safety
/// - `handle` must be a valid node handle with docs enabled
/// - `ticket` must be a valid null-terminated UTF-8 string
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_doc_join(
    handle: *const IrohNodeHandle,
    ticket: *const c_char,
    callback: IrohDocCreateCallback,
) {
    if handle.is_null() {
        let error = CString::new("handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    if ticket.is_null() {
        let error = CString::new("ticket cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    let ticket_str = match unsafe { CStr::from_ptr(ticket) }.to_str() {
        Ok(s) => s,
        Err(e) => {
            let error = CString::new(format!("Invalid ticket UTF-8: {}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    let doc_ticket: DocTicket = match ticket_str.parse() {
        Ok(t) => t,
        Err(e) => {
            let error = CString::new(format!("Invalid doc ticket: {}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    let node = unsafe { &*(handle as *const IrohNode) };

    let docs = match node.docs() {
        Some(d) => d,
        None => {
            let error = CString::new("docs not enabled on this node").unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    match node.runtime().block_on(docs.api().import(doc_ticket)) {
        Ok(doc) => {
            let namespace_id = doc.id().to_string();
            let namespace_cstr = CString::new(namespace_id).unwrap().into_raw();

            let wrapper = Box::new(DocWrapper {
                doc,
                node_handle: handle,
            });
            let doc_handle = Box::into_raw(wrapper) as *mut IrohDocHandle;

            (callback.on_success)(callback.userdata, doc_handle, namespace_cstr);
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

/// Set a key-value pair in a document.
///
/// # Safety
/// - `doc_handle` must be a valid document handle
/// - `key.data` must point to valid memory for `key.len` bytes
/// - `value.data` must point to valid memory for `value.len` bytes
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub extern "C" fn iroh_doc_set(
    doc_handle: *const IrohDocHandle,
    author_secret: IrohAuthorSecret,
    key: IrohBytes,
    value: IrohBytes,
    callback: IrohDocSetCallback,
) {
    if doc_handle.is_null() {
        let error = CString::new("doc_handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    let wrapper = unsafe { &*(doc_handle as *const DocWrapper) };
    let node = unsafe { &*(wrapper.node_handle as *const IrohNode) };

    // Reconstruct author from secret
    let author = Author::from_bytes(&author_secret.bytes);

    // Copy key and value bytes
    let key_bytes = if key.data.is_null() || key.len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(key.data, key.len).to_vec() }
    };

    let value_bytes = if value.data.is_null() || value.len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(value.data, value.len).to_vec() }
    };

    // set_bytes takes author_id (AuthorId), not Author
    let author_id = author.id();
    match node
        .runtime()
        .block_on(wrapper.doc.set_bytes(author_id, key_bytes, value_bytes))
    {
        Ok(hash) => {
            let hash: iroh_blobs::Hash = hash; // type annotation
            let hash_str = CString::new(hash.to_string()).unwrap().into_raw();
            (callback.on_success)(callback.userdata, hash_str);
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

/// Get the latest entry for a key.
///
/// # Safety
/// - `doc_handle` must be a valid document handle
/// - `key.data` must point to valid memory for `key.len` bytes
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub extern "C" fn iroh_doc_get(
    doc_handle: *const IrohDocHandle,
    key: IrohBytes,
    callback: IrohDocGetCallback,
) {
    if doc_handle.is_null() {
        let error = CString::new("doc_handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    let wrapper = unsafe { &*(doc_handle as *const DocWrapper) };
    let node = unsafe { &*(wrapper.node_handle as *const IrohNode) };

    let key_bytes = if key.data.is_null() || key.len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(key.data, key.len).to_vec() }
    };

    // Query for the exact key
    let query = iroh_docs::store::Query::key_exact(key_bytes);

    match node.runtime().block_on(async {
        use futures_lite::StreamExt;
        use std::pin::pin;
        let stream = wrapper.doc.get_many(query).await?;
        let mut stream = pin!(stream);
        // Get just the first (latest) entry
        stream.next().await.transpose()
    }) {
        Ok(Some(entry)) => {
            let ffi_entry = convert_entry_to_ffi(&entry);
            let entry_ptr = Box::into_raw(Box::new(ffi_entry));
            (callback.on_success)(callback.userdata, entry_ptr);
        }
        Ok(None) => {
            // No entry found - return null
            (callback.on_success)(callback.userdata, std::ptr::null());
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

/// Get entries by key prefix.
///
/// This streams entries back via the callback - on_entry is called for each
/// entry, then on_complete when done.
///
/// # Safety
/// - `doc_handle` must be a valid document handle
/// - `prefix.data` must point to valid memory for `prefix.len` bytes
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub extern "C" fn iroh_doc_get_many(
    doc_handle: *const IrohDocHandle,
    prefix: IrohBytes,
    callback: IrohDocGetManyCallback,
) {
    if doc_handle.is_null() {
        let error = CString::new("doc_handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    let wrapper = unsafe { &*(doc_handle as *const DocWrapper) };
    let node = unsafe { &*(wrapper.node_handle as *const IrohNode) };

    let prefix_bytes = if prefix.data.is_null() || prefix.len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(prefix.data, prefix.len).to_vec() }
    };

    // Query by prefix
    let query = iroh_docs::store::Query::key_prefix(prefix_bytes);

    match node.runtime().block_on(async {
        use futures_lite::StreamExt;
        use std::pin::pin;
        let stream = wrapper.doc.get_many(query).await?;
        let mut stream = pin!(stream);

        while let Some(result) = stream.next().await {
            match result {
                Ok(entry) => {
                    let ffi_entry = convert_entry_to_ffi(&entry);
                    let entry_ptr = Box::into_raw(Box::new(ffi_entry));
                    (callback.on_entry)(callback.userdata, entry_ptr);
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    }) {
        Ok(()) => {
            (callback.on_complete)(callback.userdata);
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

/// Delete an entry (creates a tombstone).
///
/// # Safety
/// - `doc_handle` must be a valid document handle
/// - `key.data` must point to valid memory for `key.len` bytes
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub extern "C" fn iroh_doc_del(
    doc_handle: *const IrohDocHandle,
    author_secret: IrohAuthorSecret,
    key: IrohBytes,
    callback: IrohDocDelCallback,
) {
    if doc_handle.is_null() {
        let error = CString::new("doc_handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    let wrapper = unsafe { &*(doc_handle as *const DocWrapper) };
    let node = unsafe { &*(wrapper.node_handle as *const IrohNode) };

    let author = Author::from_bytes(&author_secret.bytes);
    let author_id = author.id();

    let key_bytes = if key.data.is_null() || key.len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(key.data, key.len).to_vec() }
    };

    match node
        .runtime()
        .block_on(wrapper.doc.del(author_id, key_bytes))
    {
        Ok(count) => {
            (callback.on_success)(callback.userdata, count as u64);
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

/// Read content bytes by hash.
///
/// This fetches the actual content data for an entry (entries only contain the hash).
///
/// # Safety
/// - `handle` must be a valid node handle
/// - `content_hash` must be a valid null-terminated UTF-8 hex string
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_doc_read_content(
    handle: *const IrohNodeHandle,
    content_hash: *const c_char,
    callback: IrohGetCallback,
) {
    if handle.is_null() {
        let error = CString::new("handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    if content_hash.is_null() {
        let error = CString::new("content_hash cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    let hash_str = match unsafe { CStr::from_ptr(content_hash) }.to_str() {
        Ok(s) => s,
        Err(e) => {
            let error = CString::new(format!("Invalid hash UTF-8: {}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    let hash: iroh_blobs::Hash = match hash_str.parse() {
        Ok(h) => h,
        Err(e) => {
            let error = CString::new(format!("Invalid hash: {}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    let node = unsafe { &*(handle as *const IrohNode) };

    match node.runtime().block_on(node.store().get_bytes(hash)) {
        Ok(bytes) => {
            let mut vec = bytes.to_vec();
            let owned = IrohOwnedBytes {
                data: vec.as_mut_ptr(),
                len: vec.len(),
                capacity: vec.capacity(),
            };
            std::mem::forget(vec);
            (callback.on_success)(callback.userdata, owned);
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

/// Get a share ticket for a document.
///
/// # Safety
/// - `doc_handle` must be a valid document handle
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub extern "C" fn iroh_doc_share(
    doc_handle: *const IrohDocHandle,
    mode: IrohDocShareMode,
    callback: IrohCallback,
) {
    if doc_handle.is_null() {
        let error = CString::new("doc_handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    let wrapper = unsafe { &*(doc_handle as *const DocWrapper) };
    let node = unsafe { &*(wrapper.node_handle as *const IrohNode) };

    let share_mode = match mode {
        IrohDocShareMode::Read => ShareMode::Read,
        IrohDocShareMode::Write => ShareMode::Write,
    };

    match node.runtime().block_on(
        wrapper
            .doc
            .share(share_mode, AddrInfoOptions::RelayAndAddresses),
    ) {
        Ok(ticket) => {
            let ticket_str = CString::new(ticket.to_string()).unwrap().into_raw();
            (callback.on_success)(callback.userdata, ticket_str);
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

/// Close a document and free its resources.
///
/// # Safety
/// - `doc_handle` must be a valid document handle returned by `iroh_doc_create` or `iroh_doc_join`
/// - `doc_handle` must not be used after this call
#[unsafe(no_mangle)]
pub extern "C" fn iroh_doc_close(doc_handle: *mut IrohDocHandle) {
    if doc_handle.is_null() {
        return;
    }

    unsafe {
        // Drop the wrapper, which will drop the Doc
        drop(Box::from_raw(doc_handle as *mut DocWrapper));
    }
}

/// Free a document entry.
///
/// # Safety
/// - `entry` must be a valid entry pointer returned by document operations
/// - `entry` must not be used after this call
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_doc_entry_free(entry: *mut IrohDocEntry) {
    if entry.is_null() {
        return;
    }

    unsafe {
        let entry = Box::from_raw(entry);
        // Free the key bytes
        if !entry.key.data.is_null() {
            drop(Vec::from_raw_parts(
                entry.key.data,
                entry.key.len,
                entry.key.capacity,
            ));
        }
        // Free the content hash string
        if !entry.content_hash.is_null() {
            drop(CString::from_raw(entry.content_hash));
        }
        // The rest is stack-allocated and drops automatically
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Convert an iroh_docs Entry to FFI representation.
fn convert_entry_to_ffi(entry: &iroh_docs::Entry) -> IrohDocEntry {
    // Get author ID bytes
    let author_id = IrohAuthorId {
        bytes: entry.author().to_bytes(),
    };

    // Get key bytes (owned copy)
    let key_vec = entry.key().to_vec();
    let mut key_vec = std::mem::ManuallyDrop::new(key_vec);
    let key = IrohOwnedBytes {
        data: key_vec.as_mut_ptr(),
        len: key_vec.len(),
        capacity: key_vec.capacity(),
    };

    // Get content hash as string
    let hash_str = CString::new(entry.content_hash().to_string())
        .unwrap()
        .into_raw();

    IrohDocEntry {
        author_id,
        key,
        content_hash: hash_str,
        content_size: entry.content_len(),
        timestamp: entry.timestamp(),
    }
}

// ============================================================================
// Subscription Operations
// ============================================================================

/// Subscribe to document events.
///
/// Returns a subscription handle that can be used to cancel the subscription.
/// Events are delivered via the callback until the subscription is cancelled
/// or the stream ends.
///
/// # Safety
/// - `doc_handle` must be a valid document handle
/// - `callback` must have valid function pointers that remain valid for the
///   duration of the subscription
#[unsafe(no_mangle)]
pub extern "C" fn iroh_doc_subscribe(
    doc_handle: *const IrohDocHandle,
    callback: IrohDocSubscribeCallback,
) -> *mut IrohSubscriptionHandle {
    if doc_handle.is_null() {
        let error = CString::new("doc_handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return std::ptr::null_mut();
    }

    let wrapper = unsafe { &*(doc_handle as *const DocWrapper) };
    let node = unsafe { &*(wrapper.node_handle as *const IrohNode) };

    // Create cancellation channel
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();

    // Clone what we need for the spawned task
    let doc = wrapper.doc.clone();
    // Convert userdata to usize for Send safety (will convert back in async block)
    let userdata_addr = callback.userdata as usize;
    let on_event = callback.on_event;
    let on_complete = callback.on_complete;
    let on_failure = callback.on_failure;

    // Helper macro to convert usize back to pointer at point of use
    macro_rules! ud {
        ($addr:expr) => {
            $addr as *mut c_void
        };
    }

    // Spawn the subscription task on the node's runtime
    node.runtime().spawn(async move {
        use futures_lite::StreamExt;
        use std::pin::pin;

        // Get the subscription stream
        let stream = match doc.subscribe().await {
            Ok(s) => s,
            Err(e) => {
                let error = CString::new(format!("{:#}", e)).unwrap();
                (on_failure)(ud!(userdata_addr), error.into_raw());
                return;
            }
        };
        let mut stream = pin!(stream);

        loop {
            tokio::select! {
                // Check for cancellation
                _ = &mut cancel_rx => {
                    (on_complete)(ud!(userdata_addr));
                    break;
                }
                // Check for next event
                event = stream.next() => {
                    match event {
                        Some(Ok(live_event)) => {
                            let ffi_event = convert_live_event_to_ffi(&live_event);
                            (on_event)(ud!(userdata_addr), ffi_event);
                        }
                        Some(Err(e)) => {
                            let error = CString::new(format!("{:#}", e)).unwrap();
                            (on_failure)(ud!(userdata_addr), error.into_raw());
                            break;
                        }
                        None => {
                            // Stream ended normally
                            (on_complete)(ud!(userdata_addr));
                            break;
                        }
                    }
                }
            }
        }
    });

    // Create subscription handle
    let sub_wrapper = Box::new(SubscriptionWrapper {
        cancel_tx: Some(cancel_tx),
    });
    Box::into_raw(sub_wrapper) as *mut IrohSubscriptionHandle
}

/// Cancel an active subscription.
///
/// After calling this, no more events will be delivered and on_complete will be called.
///
/// # Safety
/// - `handle` must be a valid subscription handle returned by `iroh_doc_subscribe`
/// - `handle` must not be used after this call
#[unsafe(no_mangle)]
pub extern "C" fn iroh_subscription_cancel(handle: *mut IrohSubscriptionHandle) {
    if handle.is_null() {
        return;
    }

    unsafe {
        let mut wrapper = Box::from_raw(handle as *mut SubscriptionWrapper);
        // Send cancellation signal (if not already sent)
        if let Some(tx) = wrapper.cancel_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Free a document event.
///
/// # Safety
/// - `event` fields that are non-null must be valid pointers
#[unsafe(no_mangle)]
pub extern "C" fn iroh_doc_event_free(event: IrohDocEvent) {
    unsafe {
        // Free entry if present
        if !event.entry.is_null() {
            iroh_doc_entry_free(event.entry as *mut IrohDocEntry);
        }
        // Free peer_id if present
        if !event.peer_id.is_null() {
            drop(CString::from_raw(event.peer_id as *mut c_char));
        }
        // Free content_hash if present
        if !event.content_hash.is_null() {
            drop(CString::from_raw(event.content_hash as *mut c_char));
        }
    }
}

/// Convert a LiveEvent to FFI representation.
fn convert_live_event_to_ffi(event: &iroh_docs::engine::LiveEvent) -> IrohDocEvent {
    use iroh_docs::engine::LiveEvent;

    match event {
        LiveEvent::InsertLocal { entry } => {
            let ffi_entry = convert_entry_to_ffi(entry);
            let entry_ptr = Box::into_raw(Box::new(ffi_entry));
            IrohDocEvent {
                event_type: IrohDocEventType::InsertLocal,
                entry: entry_ptr,
                peer_id: std::ptr::null(),
                content_hash: std::ptr::null(),
            }
        }
        LiveEvent::InsertRemote { from, entry, .. } => {
            let ffi_entry = convert_entry_to_ffi(entry);
            let entry_ptr = Box::into_raw(Box::new(ffi_entry));
            let peer_id = CString::new(from.to_string()).unwrap().into_raw();
            IrohDocEvent {
                event_type: IrohDocEventType::InsertRemote,
                entry: entry_ptr,
                peer_id,
                content_hash: std::ptr::null(),
            }
        }
        LiveEvent::ContentReady { hash } => {
            let hash_str = CString::new(hash.to_string()).unwrap().into_raw();
            IrohDocEvent {
                event_type: IrohDocEventType::ContentReady,
                entry: std::ptr::null(),
                peer_id: std::ptr::null(),
                content_hash: hash_str,
            }
        }
        LiveEvent::PendingContentReady => IrohDocEvent {
            event_type: IrohDocEventType::PendingContentReady,
            entry: std::ptr::null(),
            peer_id: std::ptr::null(),
            content_hash: std::ptr::null(),
        },
        LiveEvent::NeighborUp(peer) => {
            let peer_id = CString::new(peer.to_string()).unwrap().into_raw();
            IrohDocEvent {
                event_type: IrohDocEventType::NeighborUp,
                entry: std::ptr::null(),
                peer_id,
                content_hash: std::ptr::null(),
            }
        }
        LiveEvent::NeighborDown(peer) => {
            let peer_id = CString::new(peer.to_string()).unwrap().into_raw();
            IrohDocEvent {
                event_type: IrohDocEventType::NeighborDown,
                entry: std::ptr::null(),
                peer_id,
                content_hash: std::ptr::null(),
            }
        }
        LiveEvent::SyncFinished(sync_event) => {
            let peer_id = CString::new(sync_event.peer.to_string())
                .unwrap()
                .into_raw();
            IrohDocEvent {
                event_type: IrohDocEventType::SyncFinished,
                entry: std::ptr::null(),
                peer_id,
                content_hash: std::ptr::null(),
            }
        }
    }
}

// ============================================================================
// Blob Tag Operations
// ============================================================================

/// Tag (pin) a blob to prevent garbage collection.
///
/// Tagged blobs are protected from GC until the tag is removed.
/// Use this after downloading content you want to keep.
///
/// # Safety
/// - `handle` must be a valid node handle
/// - `tag_name` must be a valid null-terminated UTF-8 string
/// - `hash_str` must be a valid null-terminated hex hash string
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_blob_tag_set(
    handle: *const IrohNodeHandle,
    tag_name: *const c_char,
    hash_str: *const c_char,
    format: IrohBlobFormat,
    callback: IrohCloseCallback,
) {
    if handle.is_null() {
        let error = CString::new("handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    if tag_name.is_null() {
        let error = CString::new("tag_name cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    if hash_str.is_null() {
        let error = CString::new("hash_str cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    let tag_name_str = match unsafe { CStr::from_ptr(tag_name) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = CString::new(format!("Invalid tag_name UTF-8: {}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    let hash_string = match unsafe { CStr::from_ptr(hash_str) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = CString::new(format!("Invalid hash UTF-8: {}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    let hash: Hash = match hash_string.parse() {
        Ok(h) => h,
        Err(e) => {
            let error = CString::new(format!("Invalid hash: {}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    let blob_format = match format {
        IrohBlobFormat::Raw => BlobFormat::Raw,
        IrohBlobFormat::HashSeq => BlobFormat::HashSeq,
    };

    let hash_and_format = HashAndFormat {
        hash,
        format: blob_format,
    };

    let node = unsafe { &*(handle as *const IrohNode) };

    // Use the store's tags API (FsStore derefs to Store which has tags())
    match node
        .runtime()
        .block_on(node.store().tags().set(tag_name_str, hash_and_format))
    {
        Ok(()) => {
            (callback.on_complete)(callback.userdata);
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

/// Create a shareable ticket for an existing local blob.
///
/// The ticket points to this node as the provider.
/// Use this to "mint" a bootstrap ticket after downloading content.
///
/// # Safety
/// - `handle` must be a valid node handle
/// - `hash_str` must be a valid null-terminated hex hash string
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_blob_ticket_create(
    handle: *const IrohNodeHandle,
    hash_str: *const c_char,
    format: IrohBlobFormat,
    callback: IrohCallback,
) {
    if handle.is_null() {
        let error = CString::new("handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    if hash_str.is_null() {
        let error = CString::new("hash_str cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    let hash_string = match unsafe { CStr::from_ptr(hash_str) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = CString::new(format!("Invalid hash UTF-8: {}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    let hash: Hash = match hash_string.parse() {
        Ok(h) => h,
        Err(e) => {
            let error = CString::new(format!("Invalid hash: {}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    let blob_format = match format {
        IrohBlobFormat::Raw => BlobFormat::Raw,
        IrohBlobFormat::HashSeq => BlobFormat::HashSeq,
    };

    let node = unsafe { &*(handle as *const IrohNode) };

    // Get the node's address and create a ticket
    let addr = node.endpoint().addr();
    let ticket = BlobTicket::new(addr, hash, blob_format);
    let ticket_str = CString::new(ticket.to_string()).unwrap().into_raw();

    (callback.on_success)(callback.userdata, ticket_str);
}

/// Remove a tag (unpin) from a blob, allowing garbage collection.
///
/// # Safety
/// - `handle` must be a valid node handle
/// - `tag_name` must be a valid null-terminated UTF-8 string
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_blob_tag_delete(
    handle: *const IrohNodeHandle,
    tag_name: *const c_char,
    callback: IrohCloseCallback,
) {
    if handle.is_null() {
        let error = CString::new("handle cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    if tag_name.is_null() {
        let error = CString::new("tag_name cannot be null").unwrap();
        (callback.on_failure)(callback.userdata, error.into_raw());
        return;
    }

    let tag_name_str = match unsafe { CStr::from_ptr(tag_name) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = CString::new(format!("Invalid tag_name UTF-8: {}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    let node = unsafe { &*(handle as *const IrohNode) };

    // Use the store's tags API to delete the tag
    match node
        .runtime()
        .block_on(node.store().tags().delete(tag_name_str))
    {
        Ok(_count) => {
            (callback.on_complete)(callback.userdata);
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

// ============================================================================
// Endpoint Identity
// ============================================================================

/// Get this node's endpoint id: the Ed25519 public key as 64 lowercase hex characters.
///
/// Same string as `IrohNodeInfo.node_id`. Returns null if `handle` is null.
/// The caller must free the result with `iroh_string_free`.
///
/// # Safety
/// - `handle` must be a valid node handle (or null)
#[unsafe(no_mangle)]
pub extern "C" fn iroh_node_endpoint_id(handle: *const IrohNodeHandle) -> *mut c_char {
    if handle.is_null() {
        return std::ptr::null_mut();
    }
    let node = unsafe { &*(handle as *const IrohNode) };
    c_string(node.endpoint_id())
}

/// Sign `message` with this node's endpoint secret key (Ed25519).
///
/// Writes the 64-byte signature to `signature_out` and returns true. Returns
/// false (writing nothing) if `handle` or `signature_out` is null. The
/// signature verifies, including under `verify_strict`, against the public key
/// returned by `iroh_node_endpoint_id`.
///
/// # Safety
/// - `handle` must be a valid node handle (or null)
/// - `message.data` must point to valid memory for `message.len` bytes (or be null with len 0)
/// - `signature_out` must point to 64 writable bytes (or be null)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_node_sign(
    handle: *const IrohNodeHandle,
    message: IrohBytes,
    signature_out: *mut u8,
) -> bool {
    if handle.is_null() || signature_out.is_null() {
        return false;
    }
    let node = unsafe { &*(handle as *const IrohNode) };
    let message = unsafe { borrowed_bytes(&message) };
    let signature = node.sign(message);
    unsafe { std::ptr::copy_nonoverlapping(signature.as_ptr(), signature_out, signature.len()) };
    true
}

/// BLAKE3 hash of `bytes` as 64 lowercase hex characters.
///
/// The caller must free the result with `iroh_string_free`.
///
/// # Safety
/// - `bytes.data` must point to valid memory for `bytes.len` bytes (or be null with len 0)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_blake3_hex(bytes: IrohBytes) -> *mut c_char {
    let data = unsafe { borrowed_bytes(&bytes) };
    c_string(crate::node::blake3_hex(data))
}

// ============================================================================
// Cancellable Operations
// ============================================================================

/// Opaque cancellation token for a spawned operation (fetch, file push).
///
/// Create with `iroh_cancel_token_new`, pass to the operation, cancel with
/// `iroh_cancel_token_cancel` (any thread, any number of times), and free with
/// `iroh_cancel_token_free` once nothing will cancel it any more. The operation
/// keeps its own reference, so freeing the token early is safe.
#[repr(C)]
pub struct IrohCancelToken {
    _private: [u8; 0],
}

/// Why a spawned operation failed.
///
/// cbindgen:prefix-with-name
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrohOperationErrorCode {
    /// Any other failure; see the message.
    Failed = 0,
    /// The remote node does not have the requested blob.
    NotFound = 1,
    /// The remote announced a blob larger than the size limit (`size` carries it).
    SizeLimitExceeded = 2,
    /// The operation was cancelled through its cancel token.
    Cancelled = 3,
    /// The node shut down while the operation was in flight.
    NodeShutDown = 4,
    /// The operation's time limit (`timeout_ms`) elapsed.
    TimedOut = 5,
}

/// Callback for `iroh_fetch_from_node`.
///
/// Exactly one of `on_success` / `on_failure` is called, once. `on_progress`
/// (nullable) is only called before that. Callbacks may run on a Tokio worker
/// thread, or synchronously inside `iroh_fetch_from_node` for argument errors.
#[repr(C)]
pub struct IrohFetchCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Cumulative progress: `(0, total)` once the size is accepted, then
    /// periodically, ending with `(total, total)`. Null to skip progress.
    pub on_progress: Option<extern "C" fn(userdata: *mut c_void, downloaded: u64, total: u64)>,
    /// Called once the verified blob has been moved to the destination path.
    pub on_success: extern "C" fn(userdata: *mut c_void, size: u64),
    /// Called on failure. `size` is the announced size for `SizeLimitExceeded`,
    /// else 0. `error` must be freed with `iroh_string_free`.
    pub on_failure: extern "C" fn(
        userdata: *mut c_void,
        code: IrohOperationErrorCode,
        size: u64,
        error: *const c_char,
    ),
}

/// Callback for `iroh_push_file_to_node`.
///
/// Exactly one of `on_success` / `on_failure` is called, once.
#[repr(C)]
pub struct IrohPushFileCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called with the blob hash as 64 lowercase hex (free with `iroh_string_free`).
    pub on_success: extern "C" fn(userdata: *mut c_void, hash: *const c_char),
    /// Called on failure. `error` must be freed with `iroh_string_free`.
    pub on_failure:
        extern "C" fn(userdata: *mut c_void, code: IrohOperationErrorCode, error: *const c_char),
}

/// Create a cancellation token. Free with `iroh_cancel_token_free`.
#[unsafe(no_mangle)]
pub extern "C" fn iroh_cancel_token_new() -> *mut IrohCancelToken {
    Box::into_raw(Box::new(tokio_util::sync::CancellationToken::new())) as *mut IrohCancelToken
}

/// Cancel the operation(s) using this token. Idempotent and thread-safe.
///
/// # Safety
/// - `token` must be a live pointer from `iroh_cancel_token_new` (or null)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_cancel_token_cancel(token: *const IrohCancelToken) {
    if let Some(token) = unsafe { cancel_token_ref(token) } {
        token.cancel();
    }
}

/// Free a cancellation token. Does not cancel anything.
///
/// # Safety
/// - `token` must come from `iroh_cancel_token_new` (or be null) and must not
///   be used after this call
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_cancel_token_free(token: *mut IrohCancelToken) {
    if !token.is_null() {
        drop(unsafe { Box::from_raw(token as *mut tokio_util::sync::CancellationToken) });
    }
}

/// Fetch the raw blob `hash_hex` from one remote node into the file at `destination_path`.
///
/// Connects to `remote_node_id_hex` (64 hex) only — never to other providers —
/// using `relay_url` and/or `direct_addrs` as address hints, and streams the
/// blob with BLAKE3 verification straight to a temporary sibling of
/// `destination_path`, which is renamed into place only after the full blob is
/// verified. The blob is not added to the local store. If the remote announces
/// a size above `max_size` the fetch fails with `SizeLimitExceeded` before any
/// content is read. On failure or cancellation no file is left at
/// `destination_path` (an existing file there is left untouched; on success it
/// is replaced).
///
/// Returns immediately; the transfer runs on the node's runtime. Cancel it with
/// `iroh_cancel_token_cancel(cancel_token)`.
///
/// # Safety
/// - `handle` must be a valid node handle that outlives the operation's callback
/// - `hash_hex`, `remote_node_id_hex`, `destination_path` must be valid
///   null-terminated UTF-8 strings
/// - `relay_url` is nullable; if non-null must be a valid null-terminated UTF-8 string
/// - `direct_addrs` is nullable; if non-null must point to `direct_addrs_len`
///   valid null-terminated UTF-8 strings
/// - `cancel_token` is nullable; if non-null must come from `iroh_cancel_token_new`
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_fetch_from_node(
    handle: *const IrohNodeHandle,
    hash_hex: *const c_char,
    remote_node_id_hex: *const c_char,
    relay_url: *const c_char,
    direct_addrs: *const *const c_char,
    direct_addrs_len: usize,
    destination_path: *const c_char,
    max_size: u64,
    cancel_token: *const IrohCancelToken,
    callback: IrohFetchCallback,
) {
    let completion = FetchCompletion {
        userdata: callback.userdata as usize,
        on_progress: callback.on_progress,
        on_success: callback.on_success,
        on_failure: callback.on_failure,
        done: false,
    };

    let parsed = (|| {
        if handle.is_null() {
            return Err("handle cannot be null".to_string());
        }
        let hash = parse_blob_hash(unsafe { required_str(hash_hex, "hash_hex") }?)?;
        let node_id = unsafe { required_str(remote_node_id_hex, "remote_node_id_hex") }?;
        let relay = unsafe { optional_str(relay_url, "relay_url") }?;
        let addrs = unsafe { str_array(direct_addrs, direct_addrs_len) }?;
        let destination =
            PathBuf::from(unsafe { required_str(destination_path, "destination_path") }?);
        let remote = crate::resolve::RemoteTarget::parse(node_id, relay, &addrs)
            .map_err(|e| format!("{:#}", e))?;
        Ok((hash, remote, destination))
    })();

    let (hash, target, destination) = match parsed {
        Ok(parsed) => parsed,
        Err(message) => {
            completion.fail(IrohOperationErrorCode::Failed, 0, message);
            return;
        }
    };

    let node = unsafe { &*(handle as *const IrohNode) };
    if node.is_shutting_down() {
        completion.fail(IrohOperationErrorCode::NodeShutDown, 0, "node shut down");
        return;
    }
    let token = operation_token(cancel_token);
    let endpoint = node.endpoint().clone();
    let shutdown = node.shutdown_token();
    let resolver = node.host_resolver().clone();
    node.runtime().spawn(async move {
        // Hostnames are resolved inside the cancellable future, so a cancel or
        // a node shutdown also ends a slow DNS lookup.
        let result = crate::node::run_cancellable(
            async {
                let remote = resolver.resolve_target(target).await?;
                crate::node::fetch_to_path(
                    &endpoint,
                    hash,
                    remote,
                    &destination,
                    max_size,
                    &token,
                    |downloaded, total| completion.progress(downloaded, total),
                )
                .await
            },
            &token,
            &shutdown,
        )
        .await;
        match result {
            Ok(size) => completion.succeed(size),
            Err(e) => {
                let (code, size) = operation_error_code(&e);
                completion.fail(code, size, e.to_string());
            }
        }
    });
}

/// Push the file at `file_path` to a remote node without loading it into memory.
///
/// The file is streamed into the local store (copied, BLAKE3/bao-encoded), then
/// pushed exactly like `iroh_push_to_node`. On success the callback receives the
/// blob hash (64 lowercase hex). The blob stays in the local store, tagged, like
/// the bytes pushed by `iroh_push_to_node`.
///
/// Returns immediately; the work runs on the node's runtime. Cancel it with
/// `iroh_cancel_token_cancel(cancel_token)`.
///
/// # Safety
/// - `handle` must be a valid node handle that outlives the operation's callback
/// - `remote_node_id_hex` and `file_path` must be valid null-terminated UTF-8 strings
/// - `relay_url` is nullable; if non-null must be a valid null-terminated UTF-8 string
/// - `direct_addrs` is nullable; if non-null must point to `direct_addrs_len`
///   valid null-terminated UTF-8 strings
/// - `cancel_token` is nullable; if non-null must come from `iroh_cancel_token_new`
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_push_file_to_node(
    handle: *const IrohNodeHandle,
    remote_node_id_hex: *const c_char,
    relay_url: *const c_char,
    direct_addrs: *const *const c_char,
    direct_addrs_len: usize,
    file_path: *const c_char,
    cancel_token: *const IrohCancelToken,
    callback: IrohPushFileCallback,
) {
    let completion = PushFileCompletion {
        userdata: callback.userdata as usize,
        on_success: callback.on_success,
        on_failure: callback.on_failure,
        done: false,
    };

    let parsed = (|| {
        if handle.is_null() {
            return Err("handle cannot be null".to_string());
        }
        let node_id = unsafe { required_str(remote_node_id_hex, "remote_node_id_hex") }?;
        let relay = unsafe { optional_str(relay_url, "relay_url") }?;
        let addrs = unsafe { str_array(direct_addrs, direct_addrs_len) }?;
        let path = PathBuf::from(unsafe { required_str(file_path, "file_path") }?);
        let remote = crate::resolve::RemoteTarget::parse(node_id, relay, &addrs)
            .map_err(|e| format!("{:#}", e))?;
        Ok((remote, path))
    })();

    let (target, path) = match parsed {
        Ok(parsed) => parsed,
        Err(message) => {
            completion.fail(IrohOperationErrorCode::Failed, message);
            return;
        }
    };

    let node = unsafe { &*(handle as *const IrohNode) };
    if node.is_shutting_down() {
        completion.fail(IrohOperationErrorCode::NodeShutDown, "node shut down");
        return;
    }
    let token = operation_token(cancel_token);
    let endpoint = node.endpoint().clone();
    let store = node.store().clone();
    let shutdown = node.shutdown_token();
    let resolver = node.host_resolver().clone();
    node.runtime().spawn(async move {
        let result = crate::node::run_cancellable(
            async {
                let remote = resolver.resolve_target(target).await?;
                crate::node::push_path(&endpoint, &store, &path, remote).await
            },
            &token,
            &shutdown,
        )
        .await;
        match result {
            Ok(hash) => completion.succeed(hash.to_hex().to_string()),
            Err(e) => {
                let (code, _) = operation_error_code(&e);
                completion.fail(code, e.to_string());
            }
        }
    });
}

/// Push `bytes` to a remote node: the cancellable form of `iroh_push_to_node` and
/// `iroh_push_to_node_with_options`, which Swift uses.
///
/// Returns immediately; the work (hostname resolution, import into the store, the
/// transfer) runs on the node's runtime. Cancel it with
/// `iroh_cancel_token_cancel(cancel_token)`: a cancel during DNS resolution or
/// the transfer stops it, and nothing is sent after a cancel. `timeout_ms` (0 = none)
/// bounds the whole operation and fails with `TimedOut`. The bytes are copied
/// before this returns. The callback is `IrohPushFileCallback`: on success the
/// blob hash (64 lowercase hex).
///
/// # Safety
/// - `handle` must be a valid node handle that outlives the operation's callback
/// - `remote_node_id_hex` must be a valid null-terminated UTF-8 string
/// - `relay_url` is nullable; if non-null must be a valid null-terminated UTF-8 string
/// - `direct_addrs` is nullable; if non-null must point to `direct_addrs_len`
///   valid null-terminated UTF-8 strings
/// - `bytes.data` must point to `bytes.len` readable bytes (copied before return)
/// - `cancel_token` is nullable; if non-null must come from `iroh_cancel_token_new`
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_push_bytes_to_node(
    handle: *const IrohNodeHandle,
    remote_node_id_hex: *const c_char,
    relay_url: *const c_char,
    direct_addrs: *const *const c_char,
    direct_addrs_len: usize,
    bytes: IrohBytes,
    timeout_ms: u64,
    cancel_token: *const IrohCancelToken,
    callback: IrohPushFileCallback,
) {
    let completion = PushFileCompletion {
        userdata: callback.userdata as usize,
        on_success: callback.on_success,
        on_failure: callback.on_failure,
        done: false,
    };

    let parsed = (|| {
        if handle.is_null() {
            return Err("handle cannot be null".to_string());
        }
        let node_id = unsafe { required_str(remote_node_id_hex, "remote_node_id_hex") }?;
        let relay = unsafe { optional_str(relay_url, "relay_url") }?;
        let addrs = unsafe { str_array(direct_addrs, direct_addrs_len) }?;
        crate::resolve::RemoteTarget::parse(node_id, relay, &addrs).map_err(|e| format!("{:#}", e))
    })();

    let target = match parsed {
        Ok(target) => target,
        Err(message) => {
            completion.fail(IrohOperationErrorCode::Failed, message);
            return;
        }
    };

    let node = unsafe { &*(handle as *const IrohNode) };
    if node.is_shutting_down() {
        completion.fail(IrohOperationErrorCode::NodeShutDown, "node shut down");
        return;
    }
    // Swift's memory may not be stable after we return.
    let data = unsafe { borrowed_bytes(&bytes) }.to_vec();
    let token = operation_token(cancel_token);
    let endpoint = node.endpoint().clone();
    let store = node.store().clone();
    let shutdown = node.shutdown_token();
    let resolver = node.host_resolver().clone();
    node.runtime().spawn(async move {
        let result = crate::node::run_cancellable(
            crate::node::push_bytes(&endpoint, &store, &resolver, target, &data, timeout_ms),
            &token,
            &shutdown,
        )
        .await;
        match result {
            Ok(hash) => completion.succeed(hash.to_hex().to_string()),
            Err(e) => {
                let (code, _) = operation_error_code(&e);
                completion.fail(code, e.to_string());
            }
        }
    });
}

/// Delivers the outcome of a fetch to its callback exactly once.
///
/// If the task is dropped before completing (the runtime shut down), `Drop`
/// reports `NodeShutDown`, so the Swift continuation is always resumed.
struct FetchCompletion {
    userdata: usize,
    on_progress: Option<extern "C" fn(userdata: *mut c_void, downloaded: u64, total: u64)>,
    on_success: extern "C" fn(userdata: *mut c_void, size: u64),
    on_failure: extern "C" fn(
        userdata: *mut c_void,
        code: IrohOperationErrorCode,
        size: u64,
        error: *const c_char,
    ),
    done: bool,
}

impl FetchCompletion {
    fn progress(&self, downloaded: u64, total: u64) {
        if let Some(on_progress) = self.on_progress {
            on_progress(self.userdata as *mut c_void, downloaded, total);
        }
    }

    fn succeed(mut self, size: u64) {
        self.done = true;
        (self.on_success)(self.userdata as *mut c_void, size);
    }

    fn fail(mut self, code: IrohOperationErrorCode, size: u64, message: impl Into<String>) {
        self.done = true;
        (self.on_failure)(
            self.userdata as *mut c_void,
            code,
            size,
            c_string(message.into()),
        );
    }
}

impl Drop for FetchCompletion {
    fn drop(&mut self) {
        if !self.done {
            self.done = true;
            (self.on_failure)(
                self.userdata as *mut c_void,
                IrohOperationErrorCode::NodeShutDown,
                0,
                c_string("node shut down".to_string()),
            );
        }
    }
}

/// Delivers the outcome of a file push to its callback exactly once (see `FetchCompletion`).
struct PushFileCompletion {
    userdata: usize,
    on_success: extern "C" fn(userdata: *mut c_void, hash: *const c_char),
    on_failure:
        extern "C" fn(userdata: *mut c_void, code: IrohOperationErrorCode, error: *const c_char),
    done: bool,
}

impl PushFileCompletion {
    fn succeed(mut self, hash_hex: String) {
        self.done = true;
        (self.on_success)(self.userdata as *mut c_void, c_string(hash_hex));
    }

    fn fail(mut self, code: IrohOperationErrorCode, message: impl Into<String>) {
        self.done = true;
        (self.on_failure)(self.userdata as *mut c_void, code, c_string(message.into()));
    }
}

impl Drop for PushFileCompletion {
    fn drop(&mut self) {
        if !self.done {
            self.done = true;
            (self.on_failure)(
                self.userdata as *mut c_void,
                IrohOperationErrorCode::NodeShutDown,
                c_string("node shut down".to_string()),
            );
        }
    }
}

fn operation_error_code(e: &crate::node::OperationError) -> (IrohOperationErrorCode, u64) {
    use crate::node::OperationError;
    match e {
        OperationError::NotFound => (IrohOperationErrorCode::NotFound, 0),
        OperationError::SizeLimitExceeded(size) => {
            (IrohOperationErrorCode::SizeLimitExceeded, *size)
        }
        OperationError::Cancelled => (IrohOperationErrorCode::Cancelled, 0),
        OperationError::NodeShutDown => (IrohOperationErrorCode::NodeShutDown, 0),
        OperationError::TimedOut => (IrohOperationErrorCode::TimedOut, 0),
        OperationError::Failed(_) => (IrohOperationErrorCode::Failed, 0),
    }
}

/// The token one operation watches: a child of the caller's token, so
/// `iroh_cancel_token_cancel` reaches it synchronously (a child of an already
/// cancelled token starts cancelled). Without a caller token it never fires.
fn operation_token(cancel_token: *const IrohCancelToken) -> tokio_util::sync::CancellationToken {
    match unsafe { cancel_token_ref(cancel_token) } {
        Some(caller) => caller.child_token(),
        None => tokio_util::sync::CancellationToken::new(),
    }
}

unsafe fn cancel_token_ref<'a>(
    token: *const IrohCancelToken,
) -> Option<&'a tokio_util::sync::CancellationToken> {
    if token.is_null() {
        None
    } else {
        Some(unsafe { &*(token as *const tokio_util::sync::CancellationToken) })
    }
}

/// Parse a blob hash given as 64 hex characters.
fn parse_blob_hash(s: &str) -> Result<Hash, String> {
    let mut bytes = [0u8; 32];
    if s.len() != 64 {
        return Err(format!(
            "Invalid hash: expected 64 hex characters, got {}",
            s.len()
        ));
    }
    hex::decode_to_slice(s, &mut bytes).map_err(|e| format!("Invalid hash: {}", e))?;
    Ok(Hash::from_bytes(bytes))
}

unsafe fn required_str<'a>(ptr: *const c_char, name: &str) -> Result<&'a str, String> {
    if ptr.is_null() {
        return Err(format!("{} cannot be null", name));
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|e| format!("Invalid {} string: {}", name, e))
}

unsafe fn optional_str<'a>(ptr: *const c_char, name: &str) -> Result<Option<&'a str>, String> {
    if ptr.is_null() {
        Ok(None)
    } else {
        unsafe { required_str(ptr, name) }.map(Some)
    }
}

unsafe fn str_array(ptr: *const *const c_char, len: usize) -> Result<Vec<String>, String> {
    if ptr.is_null() || len == 0 {
        return Ok(Vec::new());
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    let mut result = Vec::with_capacity(len);
    for &item in slice {
        if item.is_null() {
            continue;
        }
        result.push(unsafe { required_str(item, "direct address") }?.to_string());
    }
    Ok(result)
}

unsafe fn borrowed_bytes<'a>(bytes: &IrohBytes) -> &'a [u8] {
    if bytes.data.is_null() || bytes.len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(bytes.data, bytes.len) }
    }
}

/// A C string for Swift to free with `iroh_string_free` (interior NULs are dropped).
fn c_string(s: String) -> *mut c_char {
    let s = if s.contains('\0') {
        s.replace('\0', "")
    } else {
        s
    };
    CString::new(s).unwrap().into_raw()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_caller_cancel_reaches_operation_synchronously() {
        let caller = iroh_cancel_token_new();
        let operation = operation_token(caller);
        assert!(!operation.is_cancelled());
        unsafe { iroh_cancel_token_cancel(caller) };
        // No task has to run in between: the operation sees it immediately.
        assert!(operation.is_cancelled());
        unsafe { iroh_cancel_token_free(caller) };
        assert!(operation.is_cancelled());

        // Already-cancelled caller: the operation starts cancelled.
        let caller = iroh_cancel_token_new();
        unsafe { iroh_cancel_token_cancel(caller) };
        assert!(operation_token(caller).is_cancelled());
        unsafe { iroh_cancel_token_free(caller) };

        // No caller token: never cancelled by the caller.
        assert!(!operation_token(std::ptr::null()).is_cancelled());
    }

    #[test]
    fn test_parse_blob_hash_requires_64_hex() {
        let hash = Hash::new(b"abc");
        let hex = hash.to_hex().to_string();
        assert_eq!(parse_blob_hash(&hex).unwrap(), hash);
        assert_eq!(parse_blob_hash(&hex.to_uppercase()).unwrap(), hash);
        assert!(parse_blob_hash(&hex[..63]).is_err());
        assert!(parse_blob_hash(&"z".repeat(64)).is_err());
    }
}
