//! C ABI exports for Swift interop.
//!
//! This module provides the FFI boundary between Rust and Swift.
//! All functions use callback-based async patterns to integrate with
//! Swift's concurrency model.

use crate::node::IrohNode;
use iroh_blobs::ticket::BlobTicket;
use iroh_blobs::{BlobFormat, Hash, HashAndFormat};
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
        let error = c_string("storage_path cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    } else {
        let path_str = unsafe { CStr::from_ptr(config.storage_path) };
        match path_str.to_str() {
            Ok(s) => PathBuf::from(s),
            Err(e) => {
                let error = c_string(format!("Invalid storage path: {}", e));
                (callback.on_failure)(callback.userdata, error);
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
                let error = c_string(format!("Invalid custom relay URL: {}", e));
                (callback.on_failure)(callback.userdata, error);
                return;
            }
        }
    };

    let relay_enabled = config.relay_enabled;
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
            let error = c_string(format!("{:#}", e));
            (callback.on_failure)(callback.userdata, error);
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
        let error = c_string("handle cannot be null");
        (callback.on_failure)(callback.userdata, error);
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
            let ticket_cstr = c_string(ticket);
            (callback.on_success)(callback.userdata, ticket_cstr);
        }
        Err(e) => {
            let error = c_string(format!("{:#}", e));
            (callback.on_failure)(callback.userdata, error);
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
        let error = c_string("handle cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    }

    if ticket.is_null() {
        let error = c_string("ticket cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    }

    // Parse the ticket string
    let ticket_str = match unsafe { CStr::from_ptr(ticket) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = c_string(format!("Invalid ticket string: {}", e));
            (callback.on_failure)(callback.userdata, error);
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
            let error = c_string(format!("{:#}", e));
            (callback.on_failure)(callback.userdata, error);
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
        let error = c_string("handle cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    }

    if ticket.is_null() {
        let error = c_string("ticket cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    }

    // Parse the ticket string
    let ticket_str = match unsafe { CStr::from_ptr(ticket) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = c_string(format!("Invalid ticket string: {}", e));
            (callback.on_failure)(callback.userdata, error);
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
            let error = c_string(format!("{:#}", e));
            (callback.on_failure)(callback.userdata, error);
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
        let error = c_string("handle cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    }

    let node = unsafe { &*(handle as *const IrohNode) };

    match node.info() {
        Ok(info) => {
            let node_id = c_string(info.node_id);
            let relay_url = info.relay_url.map(c_string).unwrap_or(std::ptr::null_mut());

            let direct_addrs = if info.direct_addrs.is_empty() {
                std::ptr::null_mut()
            } else {
                c_string(info.direct_addrs.join("\n"))
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
            let error = c_string(format!("{:#}", e));
            (callback.on_failure)(callback.userdata, error);
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
                    let hash = c_string(parsed.hash().to_string());
                    let node_id = c_string(parsed.addr().id.to_string());

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
                let error = c_string(format!("{:#}", e));
                (on_failure)(userdata, error);
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
        let error = c_string("handle cannot be null");
        (callback.on_failure)(callback.userdata, error);
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
            let ticket_cstr = c_string(ticket);
            (callback.on_success)(callback.userdata, ticket_cstr);
        }
        Err(e) => {
            let error = c_string(format!("{:#}", e));
            (callback.on_failure)(callback.userdata, error);
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
        let error = c_string("handle cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    }

    if remote_node_id_hex.is_null() {
        let error = c_string("remote_node_id_hex cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    }

    // Parse remote node ID hex string
    let node_id_str = match unsafe { CStr::from_ptr(remote_node_id_hex) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = c_string(format!("Invalid remote_node_id_hex string: {}", e));
            (callback.on_failure)(callback.userdata, error);
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
                let error = c_string(format!("Invalid relay_url string: {}", e));
                (callback.on_failure)(callback.userdata, error);
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
                    let error = c_string(format!("Invalid direct address string: {}", e));
                    (callback.on_failure)(callback.userdata, error);
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
            let result = c_string(hash_hex);
            (callback.on_success)(callback.userdata, result);
        }
        Err(e) => {
            let error = c_string(format!("{:#}", e));
            (callback.on_failure)(callback.userdata, error);
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
        let error = c_string("handle cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    }

    if remote_node_id_hex.is_null() {
        let error = c_string("remote_node_id_hex cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    }

    // Parse remote node ID hex string
    let node_id_str = match unsafe { CStr::from_ptr(remote_node_id_hex) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = c_string(format!("Invalid remote_node_id_hex string: {}", e));
            (callback.on_failure)(callback.userdata, error);
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
                let error = c_string(format!("Invalid relay_url string: {}", e));
                (callback.on_failure)(callback.userdata, error);
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
                    let error = c_string(format!("Invalid direct address string: {}", e));
                    (callback.on_failure)(callback.userdata, error);
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
            let result = c_string(hash_hex);
            (callback.on_success)(callback.userdata, result);
        }
        Err(e) => {
            let error = c_string(format!("{:#}", e));
            (callback.on_failure)(callback.userdata, error);
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
        let error = c_string("handle cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    }

    if ticket.is_null() {
        let error = c_string("ticket cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    }

    let ticket_str = match unsafe { CStr::from_ptr(ticket) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = c_string(format!("Invalid ticket string: {}", e));
            (callback.on_failure)(callback.userdata, error);
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
            let error = c_string(format!("{:#}", e));
            (callback.on_failure)(callback.userdata, error);
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
        let error = c_string("handle cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    }

    if tag_name.is_null() {
        let error = c_string("tag_name cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    }

    if hash_str.is_null() {
        let error = c_string("hash_str cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    }

    let tag_name_str = match unsafe { CStr::from_ptr(tag_name) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = c_string(format!("Invalid tag_name UTF-8: {}", e));
            (callback.on_failure)(callback.userdata, error);
            return;
        }
    };

    let hash_string = match unsafe { CStr::from_ptr(hash_str) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = c_string(format!("Invalid hash UTF-8: {}", e));
            (callback.on_failure)(callback.userdata, error);
            return;
        }
    };

    let hash: Hash = match parse_hash_str(&hash_string) {
        Ok(h) => h,
        Err(e) => {
            let error = c_string(e);
            (callback.on_failure)(callback.userdata, error);
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
            let error = c_string(format!("{:#}", e));
            (callback.on_failure)(callback.userdata, error);
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
        let error = c_string("handle cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    }

    if hash_str.is_null() {
        let error = c_string("hash_str cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    }

    let hash_string = match unsafe { CStr::from_ptr(hash_str) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = c_string(format!("Invalid hash UTF-8: {}", e));
            (callback.on_failure)(callback.userdata, error);
            return;
        }
    };

    let hash: Hash = match parse_hash_str(&hash_string) {
        Ok(h) => h,
        Err(e) => {
            let error = c_string(e);
            (callback.on_failure)(callback.userdata, error);
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
    let ticket_str = c_string(ticket.to_string());

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
        let error = c_string("handle cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    }

    if tag_name.is_null() {
        let error = c_string("tag_name cannot be null");
        (callback.on_failure)(callback.userdata, error);
        return;
    }

    let tag_name_str = match unsafe { CStr::from_ptr(tag_name) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = c_string(format!("Invalid tag_name UTF-8: {}", e));
            (callback.on_failure)(callback.userdata, error);
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
            let error = c_string(format!("{:#}", e));
            (callback.on_failure)(callback.userdata, error);
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

/// Parse a blob hash given as 64 hex (either case) or 52 base32 characters.
///
/// `Hash::from_str` no longer panics on base32 input of the wrong length
/// (iroh-blobs 0.104, n0-computer/iroh-blobs#282), but it only takes lowercase
/// hex, so 64-character input goes through `parse_blob_hash`.
fn parse_hash_str(s: &str) -> Result<Hash, String> {
    if s.len() == 64 {
        parse_blob_hash(s)
    } else {
        s.parse().map_err(|e| format!("Invalid hash: {}", e))
    }
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
fn c_string(s: impl Into<String>) -> *mut c_char {
    let s: String = s.into();
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
    fn test_c_string_drops_interior_nul_instead_of_panicking() {
        let ptr = c_string(format!("remote said: {}bad", '\0'));
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned();
        unsafe { iroh_string_free(ptr) };
        assert_eq!(s, "remote said: bad");
    }

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

    /// RFC 4648 base32 without padding, uppercase.
    fn base32_nopad(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
        let (mut out, mut buf, mut bits) = (String::new(), 0u32, 0u32);
        for &b in bytes {
            buf = (buf << 8) | u32::from(b);
            bits += 8;
            while bits >= 5 {
                bits -= 5;
                out.push(ALPHABET[((buf >> bits) & 31) as usize] as char);
            }
        }
        if bits > 0 {
            out.push(ALPHABET[((buf << (5 - bits)) & 31) as usize] as char);
        }
        out
    }

    #[test]
    fn test_parse_hash_str_rejects_bad_input_without_panicking() {
        for s in [
            "",
            "a",
            "invalid",
            &"a".repeat(51),
            &"a".repeat(53),
            &"!".repeat(52),
        ] {
            assert!(parse_hash_str(s).is_err(), "{s:?}");
        }
        assert!(parse_hash_str(&"z".repeat(64)).is_err());
    }

    /// A panic in an `extern "C" fn` aborts the host app, so no length or
    /// alphabet may reach a panic in `Hash::from_str`.
    #[test]
    fn test_parse_hash_str_never_panics() {
        for n in 0..=128 {
            for c in ['a', 'A', 'z', '0', '9', '!', 'é'] {
                let _ = parse_hash_str(&c.to_string().repeat(n));
            }
        }
    }

    #[test]
    fn test_parse_hash_str_accepts_hex_and_base32() {
        let hash = Hash::new(b"hello");
        let base32 = base32_nopad(hash.as_bytes());
        assert_eq!(base32.len(), 52);
        assert_eq!(parse_hash_str(&hash.to_hex()).unwrap(), hash);
        assert_eq!(parse_hash_str(&hash.to_hex().to_uppercase()).unwrap(), hash);
        assert_eq!(parse_hash_str(&base32).unwrap(), hash);
        assert_eq!(parse_hash_str(&base32.to_lowercase()).unwrap(), hash);
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
