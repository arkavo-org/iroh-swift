//! C ABI exports for Swift interop.
//!
//! This module provides the FFI boundary between Rust and Swift.
//! All functions use callback-based async patterns to integrate with
//! Swift's concurrency model.

use crate::node::IrohNode;
use iroh_blobs::ticket::BlobTicket;
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
    /// Whether to use n0's public relay servers (default: true).
    pub relay_enabled: bool,
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

// ============================================================================
// Node Lifecycle
// ============================================================================

/// Create a new Iroh node asynchronously.
///
/// # Safety
/// - `config.storage_path` must be a valid null-terminated UTF-8 string
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

    let relay_enabled = config.relay_enabled;

    // Create the node synchronously
    // Note: Swift should call this from a background thread/task
    match IrohNode::new(storage_path, relay_enabled) {
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
/// This performs a graceful shutdown, ensuring pending writes are flushed.
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
        // Convert back to Box and drop it
        let node = Box::from_raw(handle as *mut IrohNode);
        // Attempt graceful shutdown, ignore errors
        let _ = node.shutdown();
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

            let ffi_info = IrohNodeInfo {
                node_id,
                relay_url,
                is_connected: info.is_connected,
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
