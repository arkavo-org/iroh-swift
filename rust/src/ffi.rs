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

/// Create a new Iroh node asynchronously on a background thread.
///
/// # Safety
/// - `config.storage_path` must be a valid null-terminated UTF-8 string
/// - `config.custom_relay_url` must be null or a valid null-terminated UTF-8 string
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub extern "C" fn iroh_node_create(config: IrohNodeConfig, callback: IrohNodeCreateCallback) {
    ffi_catch_completion(
        "iroh_node_create",
        NodeCreateCompletion::new(callback),
        |slot| {
            let parsed = unsafe { parse_node_config(&config) };
            let (
                storage_path,
                custom_relay_url,
                relay_enabled,
                serves_blobs,
                accepts_pushes,
                options,
            ) = match parsed {
                Ok(parts) => parts,
                Err(message) => {
                    slot.finish(|c| c.fail(message));
                    return;
                }
            };

            let Some(completion) = slot.take() else {
                return;
            };
            spawn_thread_or_inline("iroh-node-create", move || {
                ffi_catch_completion("iroh_node_create", completion, |slot| {
                    match IrohNode::new_with_options(
                        storage_path,
                        relay_enabled,
                        custom_relay_url,
                        serves_blobs,
                        accepts_pushes,
                        options,
                    ) {
                        Ok(node) => {
                            let boxed = Box::new(node);
                            let handle = Box::into_raw(boxed) as *mut IrohNodeHandle;
                            slot.finish(|c| c.succeed(handle));
                        }
                        Err(e) => {
                            slot.finish(|c| c.fail(format!("{:#}", e)));
                        }
                    }
                });
            });
        },
    );
}

/// Destroy an Iroh node and free its resources.
///
/// Performs shutdown on a background thread (with inline fallback on thread
/// exhaustion) to avoid blocking Swift's cooperative thread pool.
///
/// # Safety
/// - `handle` must be a valid pointer returned by `iroh_node_create`
/// - `handle` must not be used after this call
#[unsafe(no_mangle)]
pub extern "C" fn iroh_node_destroy(handle: *mut IrohNodeHandle) {
    ffi_catch_or("iroh_node_destroy", (), || {
        if handle.is_null() {
            return;
        }

        unsafe {
            let node = Box::from_raw(handle as *mut IrohNode);
            spawn_thread_or_inline("iroh-node-destroy", move || {
                let _ = node.shutdown();
            });
        }
    });
}

// ============================================================================
// Core Operations
// ============================================================================

/// Add bytes to the blob store and get a shareable ticket.
///
/// Returns immediately; the work runs on the node's Tokio runtime.
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
    ffi_catch_completion("iroh_put", StringCompletion::new(callback), |slot| {
        if handle.is_null() {
            slot.finish(|c| c.fail("handle cannot be null"));
            return;
        }

        // Copy the bytes to own them (Swift memory may not be stable)
        let data = unsafe { borrowed_bytes(&bytes) }.to_vec();
        let node = unsafe { &*(handle as *const IrohNode) };
        if node.is_shutting_down() {
            slot.finish(|c| c.fail("node shut down"));
            return;
        }

        let endpoint = node.endpoint().clone();
        let store = node.store().clone();
        let shutdown = node.shutdown_token();
        let cancel = tokio_util::sync::CancellationToken::new();
        let Some(completion) = slot.take() else {
            return;
        };
        node.runtime().spawn(async move {
            match CatchUnwindFuture(crate::node::run_cancellable(
                crate::node::put_bytes(&endpoint, &store, &data, 0),
                &cancel,
                &shutdown,
            ))
            .await
            {
                Ok(Ok(ticket)) => completion.succeed(ticket),
                Ok(Err(e)) => completion.fail(e.to_string()),
                Err(payload) => {
                    log_panic("iroh_put", payload);
                    completion.fail_panic();
                }
            }
        });
    });
}

/// Download bytes from a ticket.
///
/// Returns immediately; the work runs on the node's Tokio runtime.
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
    ffi_catch_completion("iroh_get", BytesCompletion::new(callback), |slot| {
        let ticket_str = match (|| {
            if handle.is_null() {
                return Err("handle cannot be null".to_string());
            }
            Ok(unsafe { required_str(ticket, "ticket") }?.to_string())
        })() {
            Ok(s) => s,
            Err(msg) => {
                slot.finish(|c| c.fail(msg));
                return;
            }
        };

        let node = unsafe { &*(handle as *const IrohNode) };
        if node.is_shutting_down() {
            slot.finish(|c| c.fail("node shut down"));
            return;
        }

        let endpoint = node.endpoint().clone();
        let store = node.store().clone();
        let memory_lookup = node.memory_lookup().clone();
        let shutdown = node.shutdown_token();
        let cancel = tokio_util::sync::CancellationToken::new();
        let Some(completion) = slot.take() else {
            return;
        };
        node.runtime().spawn(async move {
            match CatchUnwindFuture(crate::node::run_cancellable(
                crate::node::get_from_ticket(
                    &endpoint,
                    &store,
                    &memory_lookup,
                    &ticket_str,
                    u64::MAX,
                    0,
                    &cancel,
                    |_, _| {},
                ),
                &cancel,
                &shutdown,
            ))
            .await
            {
                Ok(Ok(bytes)) => completion.succeed(bytes),
                Ok(Err(e)) => completion.fail(e.to_string()),
                Err(payload) => {
                    log_panic("iroh_get", payload);
                    completion.fail_panic();
                }
            }
        });
    });
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
    ffi_catch_or("iroh_string_free", (), || {
        if !s.is_null() {
            unsafe {
                drop(CString::from_raw(s));
            }
        }
    });
}

/// Free bytes returned by `iroh_get`.
///
/// # Safety
/// - `bytes` must have been returned by `iroh_get`
/// - The bytes must not be used after this call
#[unsafe(no_mangle)]
pub extern "C" fn iroh_bytes_free(bytes: IrohOwnedBytes) {
    ffi_catch_or("iroh_bytes_free", (), || {
        if !bytes.data.is_null() {
            unsafe {
                // Reconstruct the Vec and let it drop
                drop(Vec::from_raw_parts(bytes.data, bytes.len, bytes.capacity));
            }
        }
    });
}

// ============================================================================
// Extended Operations
// ============================================================================

/// Download bytes from a ticket with progress reporting.
///
/// Returns immediately; the work runs on the node's Tokio runtime.
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
    ffi_catch_completion(
        "iroh_get_with_progress",
        BytesCompletion::with_progress(callback),
        |slot| {
            let ticket_str = match (|| {
                if handle.is_null() {
                    return Err("handle cannot be null".to_string());
                }
                Ok(unsafe { required_str(ticket, "ticket") }?.to_string())
            })() {
                Ok(s) => s,
                Err(msg) => {
                    slot.finish(|c| c.fail(msg));
                    return;
                }
            };

            let node = unsafe { &*(handle as *const IrohNode) };
            if node.is_shutting_down() {
                slot.finish(|c| c.fail("node shut down"));
                return;
            }

            let endpoint = node.endpoint().clone();
            let store = node.store().clone();
            let memory_lookup = node.memory_lookup().clone();
            let shutdown = node.shutdown_token();
            let cancel = tokio_util::sync::CancellationToken::new();
            let Some(completion) = slot.take() else {
                return;
            };
            node.runtime().spawn(async move {
                match CatchUnwindFuture(crate::node::run_cancellable(
                    crate::node::get_from_ticket(
                        &endpoint,
                        &store,
                        &memory_lookup,
                        &ticket_str,
                        u64::MAX,
                        0,
                        &cancel,
                        |downloaded, total| completion.progress(downloaded, total),
                    ),
                    &cancel,
                    &shutdown,
                ))
                .await
                {
                    Ok(Ok(bytes)) => completion.succeed(bytes),
                    Ok(Err(e)) => completion.fail(e.to_string()),
                    Err(payload) => {
                        log_panic("iroh_get_with_progress", payload);
                        completion.fail_panic();
                    }
                }
            });
        },
    );
}

/// Get information about the node.
///
/// Returns immediately; the work runs on the node's Tokio runtime.
///
/// # Safety
/// - `handle` must be a valid node handle
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub extern "C" fn iroh_node_info(handle: *const IrohNodeHandle, callback: IrohNodeInfoCallback) {
    ffi_catch_completion(
        "iroh_node_info",
        NodeInfoCompletion::new(callback),
        |slot| {
            if handle.is_null() {
                slot.finish(|c| c.fail("handle cannot be null"));
                return;
            }

            let node = unsafe { &*(handle as *const IrohNode) };
            if node.is_shutting_down() {
                slot.finish(|c| c.fail("node shut down"));
                return;
            }

            let endpoint = node.endpoint().clone();
            let dht_active = node.dht_active();
            let shutdown = node.shutdown_token();
            let Some(completion) = slot.take() else {
                return;
            };
            node.runtime().spawn(async move {
                match CatchUnwindFuture(async {
                    if shutdown.is_cancelled() {
                        return Err("node shut down");
                    }
                    Ok(crate::node::node_info_from_endpoint(&endpoint, dht_active))
                })
                .await
                {
                    Ok(Ok(info)) => completion.succeed(info),
                    Ok(Err(msg)) => completion.fail(msg),
                    Err(payload) => {
                        log_panic("iroh_node_info", payload);
                        completion.fail_panic();
                    }
                }
            });
        },
    );
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
    let invalid = || IrohTicketInfo {
        is_valid: false,
        hash: std::ptr::null(),
        node_id: std::ptr::null(),
        is_recursive: false,
    };
    let result = ffi_catch_or("iroh_validate_ticket", invalid(), || {
        if ticket.is_null() {
            return invalid();
        }
        match unsafe { CStr::from_ptr(ticket) }.to_str() {
            Ok(ticket_str) => match ticket_str.parse::<BlobTicket>() {
                Ok(parsed) => {
                    let hash = to_c_string(parsed.hash().to_string());
                    let node_id = to_c_string(parsed.addr().id.to_string());
                    let is_recursive = parsed.recursive();
                    IrohTicketInfo {
                        is_valid: true,
                        hash: hash.into_raw(),
                        node_id: node_id.into_raw(),
                        is_recursive,
                    }
                }
                Err(_) => invalid(),
            },
            Err(_) => invalid(),
        }
    });

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
    ffi_catch_completion("iroh_node_close", VoidCompletion::new(callback), |slot| {
        if handle.is_null() {
            slot.finish(VoidCompletion::succeed);
            return;
        }

        // Run shutdown on a dedicated thread (with inline fallback on thread
        // exhaustion) to avoid blocking Swift's cooperative thread pool.
        let node = unsafe { Box::from_raw(handle as *mut IrohNode) };
        let Some(completion) = slot.take() else {
            return;
        };
        spawn_thread_or_inline("iroh-node-close", move || {
            ffi_catch_completion("iroh_node_close", completion, |slot| {
                match node.shutdown() {
                    Ok(()) => slot.finish(VoidCompletion::succeed),
                    Err(e) => slot.finish(|c| c.fail(format!("{:#}", e))),
                }
            });
        });
    });
}

/// Add bytes to the blob store with options (e.g., timeout).
///
/// Returns immediately; the work runs on the node's Tokio runtime.
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
    ffi_catch_completion(
        "iroh_put_with_options",
        StringCompletion::new(callback),
        |slot| {
            if handle.is_null() {
                slot.finish(|c| c.fail("handle cannot be null"));
                return;
            }

            let data = unsafe { borrowed_bytes(&bytes) }.to_vec();
            let node = unsafe { &*(handle as *const IrohNode) };
            if node.is_shutting_down() {
                slot.finish(|c| c.fail("node shut down"));
                return;
            }

            let timeout_ms = options.timeout_ms;
            let endpoint = node.endpoint().clone();
            let store = node.store().clone();
            let shutdown = node.shutdown_token();
            let cancel = tokio_util::sync::CancellationToken::new();
            let Some(completion) = slot.take() else {
                return;
            };
            node.runtime().spawn(async move {
                match CatchUnwindFuture(crate::node::run_cancellable(
                    crate::node::put_bytes(&endpoint, &store, &data, timeout_ms),
                    &cancel,
                    &shutdown,
                ))
                .await
                {
                    Ok(Ok(ticket)) => completion.succeed(ticket),
                    Ok(Err(e)) => completion.fail(e.to_string()),
                    Err(payload) => {
                        log_panic("iroh_put_with_options", payload);
                        completion.fail_panic();
                    }
                }
            });
        },
    );
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
    ffi_catch_completion(
        "iroh_push_to_node",
        StringCompletion::new(callback),
        |slot| {
            let parsed = (|| {
                if handle.is_null() {
                    return Err("handle cannot be null".to_string());
                }
                let node_id =
                    unsafe { required_str(remote_node_id_hex, "remote_node_id_hex") }?.to_string();
                let relay = unsafe { optional_str(relay_url, "relay_url") }?.map(str::to_string);
                let addrs = unsafe { str_array(direct_addrs, direct_addrs_len) }?;
                Ok((node_id, relay, addrs))
            })();

            let (node_id_str, relay_url_str, addrs) = match parsed {
                Ok(parts) => parts,
                Err(msg) => {
                    slot.finish(|c| c.fail(msg));
                    return;
                }
            };

            // Copy the bytes to own them (Swift memory may not be stable)
            let data = unsafe { borrowed_bytes(&bytes) }.to_vec();
            let node = unsafe { &*(handle as *const IrohNode) };

            match node.push_to_node(&node_id_str, relay_url_str.as_deref(), &data, &addrs) {
                Ok(hash_hex) => slot.finish(|c| c.succeed(hash_hex)),
                Err(e) => slot.finish(|c| c.fail(format!("{:#}", e))),
            }
        },
    );
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
    ffi_catch_completion(
        "iroh_push_to_node_with_options",
        StringCompletion::new(callback),
        |slot| {
            let parsed = (|| {
                if handle.is_null() {
                    return Err("handle cannot be null".to_string());
                }
                let node_id =
                    unsafe { required_str(remote_node_id_hex, "remote_node_id_hex") }?.to_string();
                let relay = unsafe { optional_str(relay_url, "relay_url") }?.map(str::to_string);
                let addrs = unsafe { str_array(direct_addrs, direct_addrs_len) }?;
                Ok((node_id, relay, addrs))
            })();

            let (node_id_str, relay_url_str, addrs) = match parsed {
                Ok(parts) => parts,
                Err(msg) => {
                    slot.finish(|c| c.fail(msg));
                    return;
                }
            };

            // Copy the bytes to own them (Swift memory may not be stable)
            let data = unsafe { borrowed_bytes(&bytes) }.to_vec();
            let node = unsafe { &*(handle as *const IrohNode) };
            let timeout_ms = options.timeout_ms;

            match node.push_to_node_with_timeout(
                &node_id_str,
                relay_url_str.as_deref(),
                &data,
                &addrs,
                timeout_ms,
            ) {
                Ok(hash_hex) => slot.finish(|c| c.succeed(hash_hex)),
                Err(e) => slot.finish(|c| c.fail(format!("{:#}", e))),
            }
        },
    );
}

/// Download bytes from a ticket with options (e.g., timeout).
///
/// Returns immediately; the work runs on the node's Tokio runtime.
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
    ffi_catch_completion(
        "iroh_get_with_options",
        BytesCompletion::new(callback),
        |slot| {
            let ticket_str = match (|| {
                if handle.is_null() {
                    return Err("handle cannot be null".to_string());
                }
                Ok(unsafe { required_str(ticket, "ticket") }?.to_string())
            })() {
                Ok(s) => s,
                Err(msg) => {
                    slot.finish(|c| c.fail(msg));
                    return;
                }
            };

            let node = unsafe { &*(handle as *const IrohNode) };
            if node.is_shutting_down() {
                slot.finish(|c| c.fail("node shut down"));
                return;
            }

            let timeout_ms = options.timeout_ms;
            let endpoint = node.endpoint().clone();
            let store = node.store().clone();
            let memory_lookup = node.memory_lookup().clone();
            let shutdown = node.shutdown_token();
            let cancel = tokio_util::sync::CancellationToken::new();
            let Some(completion) = slot.take() else {
                return;
            };
            node.runtime().spawn(async move {
                match CatchUnwindFuture(crate::node::run_cancellable(
                    crate::node::get_from_ticket(
                        &endpoint,
                        &store,
                        &memory_lookup,
                        &ticket_str,
                        u64::MAX,
                        timeout_ms,
                        &cancel,
                        |_, _| {},
                    ),
                    &cancel,
                    &shutdown,
                ))
                .await
                {
                    Ok(Ok(bytes)) => completion.succeed(bytes),
                    Ok(Err(e)) => completion.fail(e.to_string()),
                    Err(payload) => {
                        log_panic("iroh_get_with_options", payload);
                        completion.fail_panic();
                    }
                }
            });
        },
    );
}

// ============================================================================
// Blob Tag Operations
// ============================================================================

/// Tag (pin) a blob to prevent garbage collection.
///
/// Tagged blobs are protected from GC until the tag is removed.
/// Returns immediately; the work runs on the node's Tokio runtime.
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
    ffi_catch_completion("iroh_blob_tag_set", VoidCompletion::new(callback), |slot| {
        let parsed = (|| {
            if handle.is_null() {
                return Err("handle cannot be null".to_string());
            }
            let tag_name_str = unsafe { required_str(tag_name, "tag_name") }?.to_string();
            let hash_string = unsafe { required_str(hash_str, "hash_str") }?;
            let hash = parse_hash_str(hash_string)?;
            Ok((tag_name_str, hash))
        })();

        let (tag_name_str, hash) = match parsed {
            Ok(parts) => parts,
            Err(msg) => {
                slot.finish(|c| c.fail(msg));
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
        if node.is_shutting_down() {
            slot.finish(|c| c.fail("node shut down"));
            return;
        }

        let store = node.store().clone();
        let shutdown = node.shutdown_token();
        let cancel = tokio_util::sync::CancellationToken::new();
        let Some(completion) = slot.take() else {
            return;
        };
        node.runtime().spawn(async move {
            match CatchUnwindFuture(crate::node::run_cancellable(
                async {
                    store
                        .tags()
                        .set(tag_name_str, hash_and_format)
                        .await
                        .map_err(|e| crate::node::OperationError::Failed(anyhow::Error::new(e)))
                },
                &cancel,
                &shutdown,
            ))
            .await
            {
                Ok(Ok(())) => completion.succeed(),
                Ok(Err(e)) => completion.fail(e.to_string()),
                Err(payload) => {
                    log_panic("iroh_blob_tag_set", payload);
                    completion.fail_panic();
                }
            }
        });
    });
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
    ffi_catch_completion(
        "iroh_blob_ticket_create",
        StringCompletion::new(callback),
        |slot| {
            let hash = match (|| {
                if handle.is_null() {
                    return Err("handle cannot be null".to_string());
                }
                if hash_str.is_null() {
                    return Err("hash_str cannot be null".to_string());
                }
                let hash_string = unsafe { CStr::from_ptr(hash_str) }
                    .to_str()
                    .map_err(|e| format!("Invalid hash UTF-8: {}", e))?;
                parse_hash_str(hash_string)
            })() {
                Ok(h) => h,
                Err(msg) => {
                    slot.finish(|c| c.fail(msg));
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
            slot.finish(|c| c.succeed(ticket.to_string()));
        },
    );
}

/// Remove a tag (unpin) from a blob, allowing garbage collection.
///
/// Returns immediately; the work runs on the node's Tokio runtime.
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
    ffi_catch_completion(
        "iroh_blob_tag_delete",
        VoidCompletion::new(callback),
        |slot| {
            let tag_name_str = match (|| {
                if handle.is_null() {
                    return Err("handle cannot be null".to_string());
                }
                Ok(unsafe { required_str(tag_name, "tag_name") }?.to_string())
            })() {
                Ok(s) => s,
                Err(msg) => {
                    slot.finish(|c| c.fail(msg));
                    return;
                }
            };

            let node = unsafe { &*(handle as *const IrohNode) };
            if node.is_shutting_down() {
                slot.finish(|c| c.fail("node shut down"));
                return;
            }

            let store = node.store().clone();
            let shutdown = node.shutdown_token();
            let cancel = tokio_util::sync::CancellationToken::new();
            let Some(completion) = slot.take() else {
                return;
            };
            node.runtime().spawn(async move {
                match CatchUnwindFuture(crate::node::run_cancellable(
                    async {
                        store
                            .tags()
                            .delete(tag_name_str)
                            .await
                            .map(|_| ())
                            .map_err(|e| crate::node::OperationError::Failed(anyhow::Error::new(e)))
                    },
                    &cancel,
                    &shutdown,
                ))
                .await
                {
                    Ok(Ok(())) => completion.succeed(),
                    Ok(Err(e)) => completion.fail(e.to_string()),
                    Err(payload) => {
                        log_panic("iroh_blob_tag_delete", payload);
                        completion.fail_panic();
                    }
                }
            });
        },
    );
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
    ffi_catch_or("iroh_node_endpoint_id", std::ptr::null_mut(), || {
        if handle.is_null() {
            return std::ptr::null_mut();
        }
        let node = unsafe { &*(handle as *const IrohNode) };
        c_string(node.endpoint_id())
    })
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
    ffi_catch_or("iroh_node_sign", false, || {
        if handle.is_null() || signature_out.is_null() {
            return false;
        }
        let node = unsafe { &*(handle as *const IrohNode) };
        let message = unsafe { borrowed_bytes(&message) };
        let signature = node.sign(message);
        unsafe {
            std::ptr::copy_nonoverlapping(signature.as_ptr(), signature_out, signature.len())
        };
        true
    })
}

/// BLAKE3 hash of `bytes` as 64 lowercase hex characters.
///
/// The caller must free the result with `iroh_string_free`.
///
/// # Safety
/// - `bytes.data` must point to valid memory for `bytes.len` bytes (or be null with len 0)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_blake3_hex(bytes: IrohBytes) -> *mut c_char {
    ffi_catch_or("iroh_blake3_hex", std::ptr::null_mut(), || {
        let data = unsafe { borrowed_bytes(&bytes) };
        c_string(crate::node::blake3_hex(data))
    })
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
    ffi_catch_or("iroh_cancel_token_new", std::ptr::null_mut(), || {
        Box::into_raw(Box::new(tokio_util::sync::CancellationToken::new())) as *mut IrohCancelToken
    })
}

/// Cancel the operation(s) using this token. Idempotent and thread-safe.
///
/// # Safety
/// - `token` must be a live pointer from `iroh_cancel_token_new` (or null)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_cancel_token_cancel(token: *const IrohCancelToken) {
    ffi_catch_or("iroh_cancel_token_cancel", (), || {
        if let Some(token) = unsafe { cancel_token_ref(token) } {
            token.cancel();
        }
    });
}

/// Free a cancellation token. Does not cancel anything.
///
/// # Safety
/// - `token` must come from `iroh_cancel_token_new` (or be null) and must not
///   be used after this call
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_cancel_token_free(token: *mut IrohCancelToken) {
    ffi_catch_or("iroh_cancel_token_free", (), || {
        if !token.is_null() {
            drop(unsafe { Box::from_raw(token as *mut tokio_util::sync::CancellationToken) });
        }
    });
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

    ffi_catch_completion("iroh_fetch_from_node", completion, |slot| {
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
                slot.finish(|c| c.fail(IrohOperationErrorCode::Failed, 0, message));
                return;
            }
        };

        let node = unsafe { &*(handle as *const IrohNode) };
        if node.is_shutting_down() {
            slot.finish(|c| c.fail(IrohOperationErrorCode::NodeShutDown, 0, "node shut down"));
            return;
        }
        let token = operation_token(cancel_token);
        let endpoint = node.endpoint().clone();
        let shutdown = node.shutdown_token();
        let resolver = node.host_resolver().clone();
        let Some(completion) = slot.take() else {
            return;
        };
        node.runtime().spawn(async move {
            // Hostnames are resolved inside the cancellable future, so a cancel or
            // a node shutdown also ends a slow DNS lookup.
            match CatchUnwindFuture(crate::node::run_cancellable(
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
            ))
            .await
            {
                Ok(Ok(size)) => completion.succeed(size),
                Ok(Err(e)) => {
                    let (code, size) = operation_error_code(&e);
                    completion.fail(code, size, e.to_string());
                }
                Err(payload) => {
                    log_panic("iroh_fetch_from_node", payload);
                    completion.fail_panic();
                }
            }
        });
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

    ffi_catch_completion("iroh_push_file_to_node", completion, |slot| {
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
                slot.finish(|c| c.fail(IrohOperationErrorCode::Failed, message));
                return;
            }
        };

        let node = unsafe { &*(handle as *const IrohNode) };
        if node.is_shutting_down() {
            slot.finish(|c| c.fail(IrohOperationErrorCode::NodeShutDown, "node shut down"));
            return;
        }
        let token = operation_token(cancel_token);
        let endpoint = node.endpoint().clone();
        let store = node.store().clone();
        let shutdown = node.shutdown_token();
        let resolver = node.host_resolver().clone();
        let Some(completion) = slot.take() else {
            return;
        };
        node.runtime().spawn(async move {
            match CatchUnwindFuture(async {
                let (hash, conn) = crate::node::run_cancellable(
                    async {
                        let remote = resolver.resolve_target(target).await?;
                        crate::node::push_path_transfer(&endpoint, &store, &path, remote).await
                    },
                    &token,
                    &shutdown,
                )
                .await?;
                crate::node::await_push_grace(conn, Some(&token), Some(&shutdown)).await?;
                Ok::<_, crate::node::OperationError>(hash.to_hex().to_string())
            })
            .await
            {
                Ok(Ok(hash_hex)) => completion.succeed(hash_hex),
                Ok(Err(e)) => {
                    let (code, _) = operation_error_code(&e);
                    completion.fail(code, e.to_string());
                }
                Err(payload) => {
                    log_panic("iroh_push_file_to_node", payload);
                    completion.fail_panic();
                }
            }
        });
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

    ffi_catch_completion("iroh_push_bytes_to_node", completion, |slot| {
        let parsed = (|| {
            if handle.is_null() {
                return Err("handle cannot be null".to_string());
            }
            let node_id = unsafe { required_str(remote_node_id_hex, "remote_node_id_hex") }?;
            let relay = unsafe { optional_str(relay_url, "relay_url") }?;
            let addrs = unsafe { str_array(direct_addrs, direct_addrs_len) }?;
            crate::resolve::RemoteTarget::parse(node_id, relay, &addrs)
                .map_err(|e| format!("{:#}", e))
        })();

        let target = match parsed {
            Ok(target) => target,
            Err(message) => {
                slot.finish(|c| c.fail(IrohOperationErrorCode::Failed, message));
                return;
            }
        };

        let node = unsafe { &*(handle as *const IrohNode) };
        if node.is_shutting_down() {
            slot.finish(|c| c.fail(IrohOperationErrorCode::NodeShutDown, "node shut down"));
            return;
        }
        // Swift's memory may not be stable after we return.
        let data = unsafe { borrowed_bytes(&bytes) }.to_vec();
        let token = operation_token(cancel_token);
        let endpoint = node.endpoint().clone();
        let store = node.store().clone();
        let shutdown = node.shutdown_token();
        let resolver = node.host_resolver().clone();
        let Some(completion) = slot.take() else {
            return;
        };
        node.runtime().spawn(async move {
            match CatchUnwindFuture(async {
                let (hash, conn) = crate::node::run_cancellable(
                    crate::node::push_bytes_transfer(
                        &endpoint, &store, &resolver, target, &data, timeout_ms,
                    ),
                    &token,
                    &shutdown,
                )
                .await?;
                crate::node::await_push_grace(conn, Some(&token), Some(&shutdown)).await?;
                Ok::<_, crate::node::OperationError>(hash.to_hex().to_string())
            })
            .await
            {
                Ok(Ok(hash_hex)) => completion.succeed(hash_hex),
                Ok(Err(e)) => {
                    let (code, _) = operation_error_code(&e);
                    completion.fail(code, e.to_string());
                }
                Err(payload) => {
                    log_panic("iroh_push_bytes_to_node", payload);
                    completion.fail_panic();
                }
            }
        });
    });
}

// ============================================================================
// Structured-Error Cancellable Operations
// ============================================================================

/// Callback for `iroh_node_create_async`.
#[repr(C)]
pub struct IrohNodeCreateAsyncCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called on success with the node handle.
    pub on_success: extern "C" fn(userdata: *mut c_void, handle: *mut IrohNodeHandle),
    /// Called on failure with structured error code and message (`iroh_string_free`).
    pub on_failure:
        extern "C" fn(userdata: *mut c_void, code: IrohOperationErrorCode, error: *const c_char),
}

/// Callback for cancellable string-returning operations (`iroh_put_async`).
#[repr(C)]
pub struct IrohOperationCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called on success with a C string (`iroh_string_free`).
    pub on_success: extern "C" fn(userdata: *mut c_void, result: *const c_char),
    /// Called on failure with structured error code and message (`iroh_string_free`).
    pub on_failure:
        extern "C" fn(userdata: *mut c_void, code: IrohOperationErrorCode, error: *const c_char),
}

/// Callback for cancellable void operations (`iroh_blob_tag_set_async`, `iroh_blob_tag_delete_async`).
#[repr(C)]
pub struct IrohVoidOperationCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called on success.
    pub on_complete: extern "C" fn(userdata: *mut c_void),
    /// Called on failure with structured error code and message (`iroh_string_free`).
    pub on_failure:
        extern "C" fn(userdata: *mut c_void, code: IrohOperationErrorCode, error: *const c_char),
}

/// Callback for `iroh_get_async`.
#[repr(C)]
pub struct IrohGetAsyncCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Optional progress callback `(downloaded, total)`. Null to skip progress.
    pub on_progress: Option<extern "C" fn(userdata: *mut c_void, downloaded: u64, total: u64)>,
    /// Called on success with owned bytes (`iroh_bytes_free`).
    pub on_success: extern "C" fn(userdata: *mut c_void, bytes: IrohOwnedBytes),
    /// Called on failure. `size` carries the announced size for `SizeLimitExceeded`, else 0.
    pub on_failure: extern "C" fn(
        userdata: *mut c_void,
        code: IrohOperationErrorCode,
        size: u64,
        error: *const c_char,
    ),
}

/// Callback for `iroh_node_info_async`.
#[repr(C)]
pub struct IrohNodeInfoAsyncCallback {
    /// Opaque pointer passed back to Swift.
    pub userdata: *mut c_void,
    /// Called on success with node info.
    pub on_success: extern "C" fn(userdata: *mut c_void, info: IrohNodeInfo),
    /// Called on failure with structured error code and message (`iroh_string_free`).
    pub on_failure:
        extern "C" fn(userdata: *mut c_void, code: IrohOperationErrorCode, error: *const c_char),
}

/// Create a new Iroh node asynchronously on a background thread with cancellation.
///
/// # Safety
/// - `config.storage_path` must be a valid null-terminated UTF-8 string
/// - `config.custom_relay_url` must be null or a valid null-terminated UTF-8 string
/// - `cancel_token` is nullable; if non-null must come from `iroh_cancel_token_new`
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_node_create_async(
    config: IrohNodeConfig,
    cancel_token: *const IrohCancelToken,
    callback: IrohNodeCreateAsyncCallback,
) {
    ffi_catch_completion(
        "iroh_node_create_async",
        NodeCreateAsyncCompletion::new(callback),
        |slot| {
            let parsed = unsafe { parse_node_config(&config) };
            let (
                storage_path,
                custom_relay_url,
                relay_enabled,
                serves_blobs,
                accepts_pushes,
                options,
            ) = match parsed {
                Ok(parts) => parts,
                Err(message) => {
                    slot.finish(|c| c.fail(IrohOperationErrorCode::Failed, message));
                    return;
                }
            };

            let token = operation_token(cancel_token);
            if token.is_cancelled() {
                slot.finish(|c| c.fail(IrohOperationErrorCode::Cancelled, "operation cancelled"));
                return;
            }

            let Some(completion) = slot.take() else {
                return;
            };
            spawn_thread_or_inline("iroh-node-create", move || {
                ffi_catch_completion("iroh_node_create_async", completion, |slot| {
                    match IrohNode::new_with_options_cancellable(
                        storage_path,
                        relay_enabled,
                        custom_relay_url,
                        serves_blobs,
                        accepts_pushes,
                        options,
                        &token,
                    ) {
                        Ok(node) => {
                            let boxed = Box::new(node);
                            let handle = Box::into_raw(boxed) as *mut IrohNodeHandle;
                            slot.finish(|c| c.succeed(handle));
                        }
                        Err(e) => {
                            let (code, _) = operation_error_code(&e);
                            slot.finish(|c| c.fail(code, e.to_string()));
                        }
                    }
                });
            });
        },
    );
}

/// Add bytes to the blob store asynchronously with optional timeout and cancellation.
///
/// `timeout_ms` of 0 means no timeout.
///
/// # Safety
/// - `handle` must be a valid node handle that outlives the operation's callback
/// - `bytes.data` must point to `bytes.len` readable bytes (copied before return)
/// - `cancel_token` is nullable; if non-null must come from `iroh_cancel_token_new`
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_put_async(
    handle: *const IrohNodeHandle,
    bytes: IrohBytes,
    timeout_ms: u64,
    cancel_token: *const IrohCancelToken,
    callback: IrohOperationCallback,
) {
    let completion = OperationStringCompletion {
        userdata: callback.userdata as usize,
        on_success: callback.on_success,
        on_failure: callback.on_failure,
        done: false,
    };

    ffi_catch_completion("iroh_put_async", completion, |slot| {
        if handle.is_null() {
            slot.finish(|c| c.fail(IrohOperationErrorCode::Failed, "handle cannot be null"));
            return;
        }

        let node = unsafe { &*(handle as *const IrohNode) };
        if node.is_shutting_down() {
            slot.finish(|c| c.fail(IrohOperationErrorCode::NodeShutDown, "node shut down"));
            return;
        }

        let data = unsafe { borrowed_bytes(&bytes) }.to_vec();
        let token = operation_token(cancel_token);
        let endpoint = node.endpoint().clone();
        let store = node.store().clone();
        let shutdown = node.shutdown_token();
        let Some(completion) = slot.take() else {
            return;
        };
        node.runtime().spawn(async move {
            match CatchUnwindFuture(crate::node::run_cancellable(
                crate::node::put_bytes(&endpoint, &store, &data, timeout_ms),
                &token,
                &shutdown,
            ))
            .await
            {
                Ok(Ok(ticket)) => completion.succeed(ticket),
                Ok(Err(e)) => {
                    let (code, _) = operation_error_code(&e);
                    completion.fail(code, e.to_string());
                }
                Err(payload) => {
                    log_panic("iroh_put_async", payload);
                    completion.fail_panic();
                }
            }
        });
    });
}

/// Download bytes from a ticket asynchronously with `max_size`, optional timeout,
/// optional progress callback, and cancellation.
///
/// `timeout_ms` of 0 means no timeout; `max_size` of `u64::MAX` means no size limit.
///
/// # Safety
/// - `handle` must be a valid node handle that outlives the operation's callback
/// - `ticket` must be a valid null-terminated UTF-8 string
/// - `cancel_token` is nullable; if non-null must come from `iroh_cancel_token_new`
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_get_async(
    handle: *const IrohNodeHandle,
    ticket: *const c_char,
    timeout_ms: u64,
    max_size: u64,
    cancel_token: *const IrohCancelToken,
    callback: IrohGetAsyncCallback,
) {
    let completion = GetAsyncCompletion {
        userdata: callback.userdata as usize,
        on_progress: callback.on_progress,
        on_success: callback.on_success,
        on_failure: callback.on_failure,
        done: false,
    };

    ffi_catch_completion("iroh_get_async", completion, |slot| {
        let ticket_str = match (|| {
            if handle.is_null() {
                return Err("handle cannot be null".to_string());
            }
            Ok(unsafe { required_str(ticket, "ticket") }?.to_string())
        })() {
            Ok(s) => s,
            Err(message) => {
                slot.finish(|c| c.fail(IrohOperationErrorCode::Failed, 0, message));
                return;
            }
        };

        let node = unsafe { &*(handle as *const IrohNode) };
        if node.is_shutting_down() {
            slot.finish(|c| c.fail(IrohOperationErrorCode::NodeShutDown, 0, "node shut down"));
            return;
        }

        let token = operation_token(cancel_token);
        let endpoint = node.endpoint().clone();
        let store = node.store().clone();
        let memory_lookup = node.memory_lookup().clone();
        let shutdown = node.shutdown_token();
        let Some(completion) = slot.take() else {
            return;
        };
        node.runtime().spawn(async move {
            match CatchUnwindFuture(crate::node::run_cancellable(
                crate::node::get_from_ticket(
                    &endpoint,
                    &store,
                    &memory_lookup,
                    &ticket_str,
                    max_size,
                    timeout_ms,
                    &token,
                    |downloaded, total| completion.progress(downloaded, total),
                ),
                &token,
                &shutdown,
            ))
            .await
            {
                Ok(Ok(bytes)) => completion.succeed(bytes),
                Ok(Err(e)) => {
                    let (code, size) = operation_error_code(&e);
                    completion.fail(code, size, e.to_string());
                }
                Err(payload) => {
                    log_panic("iroh_get_async", payload);
                    completion.fail_panic();
                }
            }
        });
    });
}

/// Tag (pin) a blob asynchronously with cancellation.
///
/// # Safety
/// - `handle` must be a valid node handle that outlives the operation's callback
/// - `tag_name` and `hash_str` must be valid null-terminated UTF-8 strings
/// - `cancel_token` is nullable; if non-null must come from `iroh_cancel_token_new`
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_blob_tag_set_async(
    handle: *const IrohNodeHandle,
    tag_name: *const c_char,
    hash_str: *const c_char,
    format: IrohBlobFormat,
    cancel_token: *const IrohCancelToken,
    callback: IrohVoidOperationCallback,
) {
    let completion = VoidOperationCompletion {
        userdata: callback.userdata as usize,
        on_complete: callback.on_complete,
        on_failure: callback.on_failure,
        done: false,
    };

    ffi_catch_completion("iroh_blob_tag_set_async", completion, |slot| {
        let parsed = (|| {
            if handle.is_null() {
                return Err("handle cannot be null".to_string());
            }
            let tag_name_str = unsafe { required_str(tag_name, "tag_name") }?.to_string();
            let hash_string = unsafe { required_str(hash_str, "hash_str") }?;
            let hash = parse_hash_str(hash_string)?;
            Ok((tag_name_str, hash))
        })();

        let (tag_name_str, hash) = match parsed {
            Ok(parts) => parts,
            Err(message) => {
                slot.finish(|c| c.fail(IrohOperationErrorCode::Failed, message));
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
        if node.is_shutting_down() {
            slot.finish(|c| c.fail(IrohOperationErrorCode::NodeShutDown, "node shut down"));
            return;
        }

        let token = operation_token(cancel_token);
        let store = node.store().clone();
        let shutdown = node.shutdown_token();
        let Some(completion) = slot.take() else {
            return;
        };
        node.runtime().spawn(async move {
            match CatchUnwindFuture(crate::node::run_cancellable(
                async {
                    store
                        .tags()
                        .set(tag_name_str, hash_and_format)
                        .await
                        .map_err(|e| crate::node::OperationError::Failed(anyhow::Error::new(e)))
                },
                &token,
                &shutdown,
            ))
            .await
            {
                Ok(Ok(())) => completion.succeed(),
                Ok(Err(e)) => {
                    let (code, _) = operation_error_code(&e);
                    completion.fail(code, e.to_string());
                }
                Err(payload) => {
                    log_panic("iroh_blob_tag_set_async", payload);
                    completion.fail_panic();
                }
            }
        });
    });
}

/// Remove a tag (unpin) asynchronously with cancellation.
///
/// # Safety
/// - `handle` must be a valid node handle that outlives the operation's callback
/// - `tag_name` must be a valid null-terminated UTF-8 string
/// - `cancel_token` is nullable; if non-null must come from `iroh_cancel_token_new`
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_blob_tag_delete_async(
    handle: *const IrohNodeHandle,
    tag_name: *const c_char,
    cancel_token: *const IrohCancelToken,
    callback: IrohVoidOperationCallback,
) {
    let completion = VoidOperationCompletion {
        userdata: callback.userdata as usize,
        on_complete: callback.on_complete,
        on_failure: callback.on_failure,
        done: false,
    };

    ffi_catch_completion("iroh_blob_tag_delete_async", completion, |slot| {
        let tag_name_str = match (|| {
            if handle.is_null() {
                return Err("handle cannot be null".to_string());
            }
            Ok(unsafe { required_str(tag_name, "tag_name") }?.to_string())
        })() {
            Ok(s) => s,
            Err(message) => {
                slot.finish(|c| c.fail(IrohOperationErrorCode::Failed, message));
                return;
            }
        };

        let node = unsafe { &*(handle as *const IrohNode) };
        if node.is_shutting_down() {
            slot.finish(|c| c.fail(IrohOperationErrorCode::NodeShutDown, "node shut down"));
            return;
        }

        let token = operation_token(cancel_token);
        let store = node.store().clone();
        let shutdown = node.shutdown_token();
        let Some(completion) = slot.take() else {
            return;
        };
        node.runtime().spawn(async move {
            match CatchUnwindFuture(crate::node::run_cancellable(
                async {
                    store
                        .tags()
                        .delete(tag_name_str)
                        .await
                        .map(|_| ())
                        .map_err(|e| crate::node::OperationError::Failed(anyhow::Error::new(e)))
                },
                &token,
                &shutdown,
            ))
            .await
            {
                Ok(Ok(())) => completion.succeed(),
                Ok(Err(e)) => {
                    let (code, _) = operation_error_code(&e);
                    completion.fail(code, e.to_string());
                }
                Err(payload) => {
                    log_panic("iroh_blob_tag_delete_async", payload);
                    completion.fail_panic();
                }
            }
        });
    });
}

/// Get information about the node asynchronously with cancellation.
///
/// # Safety
/// - `handle` must be a valid node handle that outlives the operation's callback
/// - `cancel_token` is nullable; if non-null must come from `iroh_cancel_token_new`
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_node_info_async(
    handle: *const IrohNodeHandle,
    cancel_token: *const IrohCancelToken,
    callback: IrohNodeInfoAsyncCallback,
) {
    let completion = NodeInfoAsyncCompletion {
        userdata: callback.userdata as usize,
        on_success: callback.on_success,
        on_failure: callback.on_failure,
        done: false,
    };

    ffi_catch_completion("iroh_node_info_async", completion, |slot| {
        if handle.is_null() {
            slot.finish(|c| c.fail(IrohOperationErrorCode::Failed, "handle cannot be null"));
            return;
        }

        let node = unsafe { &*(handle as *const IrohNode) };
        if node.is_shutting_down() {
            slot.finish(|c| c.fail(IrohOperationErrorCode::NodeShutDown, "node shut down"));
            return;
        }

        let token = operation_token(cancel_token);
        let endpoint = node.endpoint().clone();
        let dht_active = node.dht_active();
        let shutdown = node.shutdown_token();
        let Some(completion) = slot.take() else {
            return;
        };
        node.runtime().spawn(async move {
            match CatchUnwindFuture(crate::node::run_cancellable(
                async { Ok(crate::node::node_info_from_endpoint(&endpoint, dht_active)) },
                &token,
                &shutdown,
            ))
            .await
            {
                Ok(Ok(info)) => completion.succeed(info),
                Ok(Err(e)) => {
                    let (code, _) = operation_error_code(&e);
                    completion.fail(code, e.to_string());
                }
                Err(payload) => {
                    log_panic("iroh_node_info_async", payload);
                    completion.fail_panic();
                }
            }
        });
    });
}

// ============================================================================
// Panic Boundary Helpers
// ============================================================================

const INTERNAL_PANIC_MESSAGE: &str = "internal panic";

/// Log a caught panic without exposing raw panic payloads (which may contain
/// paths, tickets, or node IDs) across the FFI boundary.
fn log_panic(export: &'static str, payload: Box<dyn std::any::Any + Send>) {
    tracing::error!(export, "{INTERNAL_PANIC_MESSAGE}");
    if let Some(detail) = payload.downcast_ref::<&str>() {
        tracing::debug!(export, detail, "panic payload");
    } else if let Some(detail) = payload.downcast_ref::<String>() {
        tracing::debug!(export, detail = %detail, "panic payload");
    }
}

/// Catch any unwind at an `extern "C"` boundary, logging and returning `fallback`.
fn ffi_catch_or<T>(export: &'static str, fallback: T, f: impl FnOnce() -> T) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(payload) => {
            log_panic(export, payload);
            fallback
        }
    }
}

/// Future wrapper that catches panics thrown during `Future::poll` so a task
/// can explicitly report `Failed` / `"internal panic"` instead of letting the
/// completion guard's `Drop` (which reports `NodeShutDown`) conflate task panics
/// with runtime shutdown.
struct CatchUnwindFuture<F>(F);

impl<F: std::future::Future> std::future::Future for CatchUnwindFuture<F> {
    type Output = Result<F::Output, Box<dyn std::any::Any + Send>>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // SAFETY: structural projection; `self` is never moved out of.
        let inner = unsafe { self.map_unchecked_mut(|s| &mut s.0) };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| inner.poll(cx))) {
            Ok(std::task::Poll::Ready(val)) => std::task::Poll::Ready(Ok(val)),
            Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
            Err(payload) => std::task::Poll::Ready(Err(payload)),
        }
    }
}

/// Trait implemented by every FFI completion drop-guard so `ffi_catch_completion`
/// and `CatchUnwindFuture` can report a caught panic through the export's exact
/// failure path with a fixed `"internal panic"` message.
trait FailOnPanic {
    fn fail_panic(self);
}

/// Owns a completion guard inside `ffi_catch_completion` so it is consumed at
/// most once even if a panic occurs after the callback has already fired or
/// been moved into a background thread/task.
struct CompletionSlot<C>(Option<C>);

impl<C> CompletionSlot<C> {
    fn take(&mut self) -> Option<C> {
        self.0.take()
    }

    fn finish(&mut self, f: impl FnOnce(C)) {
        if let Some(completion) = self.0.take() {
            f(completion);
        }
    }
}

/// Run the body of a callback-based `extern "C"` export inside `catch_unwind`.
///
/// If the body panics before taking the completion guard from `slot`, a fixed
/// `"internal panic"` failure is reported through `C::fail_panic` exactly once.
/// If the guard was already consumed (callback already invoked or moved to a
/// spawned worker), a subsequent panic is caught and logged without invoking
/// the callback again.
fn ffi_catch_completion<C: FailOnPanic>(
    export: &'static str,
    completion: C,
    f: impl FnOnce(&mut CompletionSlot<C>),
) {
    let mut slot = CompletionSlot(Some(completion));
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        f(&mut slot);
    })) {
        log_panic(export, payload);
        if let Some(completion) = slot.take() {
            completion.fail_panic();
        }
    }
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
        let err = c_string(message.into());
        self.done = true;
        (self.on_failure)(self.userdata as *mut c_void, code, size, err);
    }
}

impl FailOnPanic for FetchCompletion {
    fn fail_panic(self) {
        self.fail(IrohOperationErrorCode::Failed, 0, INTERNAL_PANIC_MESSAGE);
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
                c_string("node shut down"),
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
        let hash = c_string(hash_hex);
        self.done = true;
        (self.on_success)(self.userdata as *mut c_void, hash);
    }

    fn fail(mut self, code: IrohOperationErrorCode, message: impl Into<String>) {
        let err = c_string(message.into());
        self.done = true;
        (self.on_failure)(self.userdata as *mut c_void, code, err);
    }
}

impl FailOnPanic for PushFileCompletion {
    fn fail_panic(self) {
        self.fail(IrohOperationErrorCode::Failed, INTERNAL_PANIC_MESSAGE);
    }
}

impl Drop for PushFileCompletion {
    fn drop(&mut self) {
        if !self.done {
            self.done = true;
            (self.on_failure)(
                self.userdata as *mut c_void,
                IrohOperationErrorCode::NodeShutDown,
                c_string("node shut down"),
            );
        }
    }
}

/// Spawn `f` on a named OS thread; if thread creation fails (e.g. thread
/// exhaustion), execute `f` inline instead of panicking across `extern "C"`.
fn spawn_thread_or_inline<F>(name: &'static str, f: F)
where
    F: FnOnce() + Send + 'static,
{
    let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(f)));
    let thread_slot = std::sync::Arc::clone(&slot);
    let spawned = std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            if let Some(work) = thread_slot.lock().unwrap_or_else(|e| e.into_inner()).take()
                && let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
            {
                log_panic(name, payload);
            }
        });
    if spawned.is_err()
        && let Some(work) = slot.lock().unwrap_or_else(|e| e.into_inner()).take()
        && let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
    {
        log_panic(name, payload);
    }
}

type ParsedNodeConfig = (
    PathBuf,
    Option<String>,
    bool,
    bool,
    bool,
    crate::node::NodeOptions,
);

unsafe fn parse_node_config(config: &IrohNodeConfig) -> Result<ParsedNodeConfig, String> {
    let storage_path = PathBuf::from(unsafe { required_str(config.storage_path, "storage_path") }?);
    let custom_relay_url =
        unsafe { optional_str(config.custom_relay_url, "custom_relay_url") }?.map(str::to_string);
    let options = crate::node::NodeOptions {
        dht_lookup: config.dht_lookup,
        dht_publish: config.dht_publish,
        ..Default::default()
    };
    Ok((
        storage_path,
        custom_relay_url,
        config.relay_enabled,
        config.serves_blobs,
        config.accepts_pushes,
        options,
    ))
}

fn into_owned_bytes(mut vec: Vec<u8>) -> IrohOwnedBytes {
    let owned = IrohOwnedBytes {
        data: vec.as_mut_ptr(),
        len: vec.len(),
        capacity: vec.capacity(),
    };
    std::mem::forget(vec);
    owned
}

fn ffi_node_info(info: crate::node::NodeInfo) -> IrohNodeInfo {
    let node_id = c_string(info.node_id);
    let relay_url = info.relay_url.map(c_string).unwrap_or(std::ptr::null_mut());
    let direct_addrs = if info.direct_addrs.is_empty() {
        std::ptr::null_mut()
    } else {
        c_string(info.direct_addrs.join("\n"))
    };
    IrohNodeInfo {
        node_id,
        relay_url,
        is_connected: info.is_connected,
        direct_addrs,
        dht_active: info.dht_active,
    }
}

/// Drop-guard completion for `IrohNodeCreateCallback`.
struct NodeCreateCompletion {
    userdata: usize,
    on_success: extern "C" fn(userdata: *mut c_void, handle: *mut IrohNodeHandle),
    on_failure: extern "C" fn(userdata: *mut c_void, error: *const c_char),
    done: bool,
}

impl NodeCreateCompletion {
    fn new(callback: IrohNodeCreateCallback) -> Self {
        Self {
            userdata: callback.userdata as usize,
            on_success: callback.on_success,
            on_failure: callback.on_failure,
            done: false,
        }
    }

    fn succeed(mut self, handle: *mut IrohNodeHandle) {
        self.done = true;
        (self.on_success)(self.userdata as *mut c_void, handle);
    }

    fn fail(mut self, message: impl Into<String>) {
        let err = c_string(message.into());
        self.done = true;
        (self.on_failure)(self.userdata as *mut c_void, err);
    }
}

impl FailOnPanic for NodeCreateCompletion {
    fn fail_panic(self) {
        self.fail(INTERNAL_PANIC_MESSAGE);
    }
}

impl Drop for NodeCreateCompletion {
    fn drop(&mut self) {
        if !self.done {
            self.done = true;
            (self.on_failure)(
                self.userdata as *mut c_void,
                c_string("node creation aborted"),
            );
        }
    }
}

/// Drop-guard completion for `IrohNodeCreateAsyncCallback`.
struct NodeCreateAsyncCompletion {
    userdata: usize,
    on_success: extern "C" fn(userdata: *mut c_void, handle: *mut IrohNodeHandle),
    on_failure:
        extern "C" fn(userdata: *mut c_void, code: IrohOperationErrorCode, error: *const c_char),
    done: bool,
}

impl NodeCreateAsyncCompletion {
    fn new(callback: IrohNodeCreateAsyncCallback) -> Self {
        Self {
            userdata: callback.userdata as usize,
            on_success: callback.on_success,
            on_failure: callback.on_failure,
            done: false,
        }
    }

    fn succeed(mut self, handle: *mut IrohNodeHandle) {
        self.done = true;
        (self.on_success)(self.userdata as *mut c_void, handle);
    }

    fn fail(mut self, code: IrohOperationErrorCode, message: impl Into<String>) {
        let err = c_string(message.into());
        self.done = true;
        (self.on_failure)(self.userdata as *mut c_void, code, err);
    }
}

impl FailOnPanic for NodeCreateAsyncCompletion {
    fn fail_panic(self) {
        self.fail(IrohOperationErrorCode::Failed, INTERNAL_PANIC_MESSAGE);
    }
}

impl Drop for NodeCreateAsyncCompletion {
    fn drop(&mut self) {
        if !self.done {
            self.done = true;
            (self.on_failure)(
                self.userdata as *mut c_void,
                IrohOperationErrorCode::Failed,
                c_string("node creation aborted"),
            );
        }
    }
}

/// Drop-guard completion for legacy `IrohCallback`.
struct StringCompletion {
    userdata: usize,
    on_success: extern "C" fn(userdata: *mut c_void, result: *const c_char),
    on_failure: extern "C" fn(userdata: *mut c_void, error: *const c_char),
    done: bool,
}

impl StringCompletion {
    fn new(callback: IrohCallback) -> Self {
        Self {
            userdata: callback.userdata as usize,
            on_success: callback.on_success,
            on_failure: callback.on_failure,
            done: false,
        }
    }

    fn succeed(mut self, value: String) {
        let result = c_string(value);
        self.done = true;
        (self.on_success)(self.userdata as *mut c_void, result);
    }

    fn fail(mut self, message: impl Into<String>) {
        let err = c_string(message.into());
        self.done = true;
        (self.on_failure)(self.userdata as *mut c_void, err);
    }
}

impl FailOnPanic for StringCompletion {
    fn fail_panic(self) {
        self.fail(INTERNAL_PANIC_MESSAGE);
    }
}

impl Drop for StringCompletion {
    fn drop(&mut self) {
        if !self.done {
            self.done = true;
            (self.on_failure)(self.userdata as *mut c_void, c_string("node shut down"));
        }
    }
}

/// Drop-guard completion for legacy `IrohGetCallback` / `IrohGetProgressCallback`.
struct BytesCompletion {
    userdata: usize,
    on_progress: Option<extern "C" fn(userdata: *mut c_void, progress: IrohDownloadProgress)>,
    on_success: extern "C" fn(userdata: *mut c_void, bytes: IrohOwnedBytes),
    on_failure: extern "C" fn(userdata: *mut c_void, error: *const c_char),
    done: bool,
}

impl BytesCompletion {
    fn new(callback: IrohGetCallback) -> Self {
        Self {
            userdata: callback.userdata as usize,
            on_progress: None,
            on_success: callback.on_success,
            on_failure: callback.on_failure,
            done: false,
        }
    }

    fn with_progress(callback: IrohGetProgressCallback) -> Self {
        Self {
            userdata: callback.userdata as usize,
            on_progress: Some(callback.on_progress),
            on_success: callback.on_success,
            on_failure: callback.on_failure,
            done: false,
        }
    }

    fn progress(&self, downloaded: u64, total: u64) {
        if let Some(on_progress) = self.on_progress {
            on_progress(
                self.userdata as *mut c_void,
                IrohDownloadProgress { downloaded, total },
            );
        }
    }

    fn succeed(mut self, bytes: Vec<u8>) {
        let owned = into_owned_bytes(bytes);
        self.done = true;
        (self.on_success)(self.userdata as *mut c_void, owned);
    }

    fn fail(mut self, message: impl Into<String>) {
        let err = c_string(message.into());
        self.done = true;
        (self.on_failure)(self.userdata as *mut c_void, err);
    }
}

impl FailOnPanic for BytesCompletion {
    fn fail_panic(self) {
        self.fail(INTERNAL_PANIC_MESSAGE);
    }
}

impl Drop for BytesCompletion {
    fn drop(&mut self) {
        if !self.done {
            self.done = true;
            (self.on_failure)(self.userdata as *mut c_void, c_string("node shut down"));
        }
    }
}

/// Drop-guard completion for legacy `IrohCloseCallback` (used by `iroh_blob_tag_*`).
struct VoidCompletion {
    userdata: usize,
    on_complete: extern "C" fn(userdata: *mut c_void),
    on_failure: extern "C" fn(userdata: *mut c_void, error: *const c_char),
    done: bool,
}

impl VoidCompletion {
    fn new(callback: IrohCloseCallback) -> Self {
        Self {
            userdata: callback.userdata as usize,
            on_complete: callback.on_complete,
            on_failure: callback.on_failure,
            done: false,
        }
    }

    fn succeed(mut self) {
        self.done = true;
        (self.on_complete)(self.userdata as *mut c_void);
    }

    fn fail(mut self, message: impl Into<String>) {
        let err = c_string(message.into());
        self.done = true;
        (self.on_failure)(self.userdata as *mut c_void, err);
    }
}

impl FailOnPanic for VoidCompletion {
    fn fail_panic(self) {
        self.fail(INTERNAL_PANIC_MESSAGE);
    }
}

impl Drop for VoidCompletion {
    fn drop(&mut self) {
        if !self.done {
            self.done = true;
            (self.on_failure)(self.userdata as *mut c_void, c_string("node shut down"));
        }
    }
}

/// Drop-guard completion for legacy `IrohNodeInfoCallback`.
struct NodeInfoCompletion {
    userdata: usize,
    on_success: extern "C" fn(userdata: *mut c_void, info: IrohNodeInfo),
    on_failure: extern "C" fn(userdata: *mut c_void, error: *const c_char),
    done: bool,
}

impl NodeInfoCompletion {
    fn new(callback: IrohNodeInfoCallback) -> Self {
        Self {
            userdata: callback.userdata as usize,
            on_success: callback.on_success,
            on_failure: callback.on_failure,
            done: false,
        }
    }

    fn succeed(mut self, info: crate::node::NodeInfo) {
        let info = ffi_node_info(info);
        self.done = true;
        (self.on_success)(self.userdata as *mut c_void, info);
    }

    fn fail(mut self, message: impl Into<String>) {
        let err = c_string(message.into());
        self.done = true;
        (self.on_failure)(self.userdata as *mut c_void, err);
    }
}

impl FailOnPanic for NodeInfoCompletion {
    fn fail_panic(self) {
        self.fail(INTERNAL_PANIC_MESSAGE);
    }
}

impl Drop for NodeInfoCompletion {
    fn drop(&mut self) {
        if !self.done {
            self.done = true;
            (self.on_failure)(self.userdata as *mut c_void, c_string("node shut down"));
        }
    }
}

/// Drop-guard completion for `IrohOperationCallback`.
struct OperationStringCompletion {
    userdata: usize,
    on_success: extern "C" fn(userdata: *mut c_void, result: *const c_char),
    on_failure:
        extern "C" fn(userdata: *mut c_void, code: IrohOperationErrorCode, error: *const c_char),
    done: bool,
}

impl OperationStringCompletion {
    fn succeed(mut self, value: String) {
        let result = c_string(value);
        self.done = true;
        (self.on_success)(self.userdata as *mut c_void, result);
    }

    fn fail(mut self, code: IrohOperationErrorCode, message: impl Into<String>) {
        let err = c_string(message.into());
        self.done = true;
        (self.on_failure)(self.userdata as *mut c_void, code, err);
    }
}

impl FailOnPanic for OperationStringCompletion {
    fn fail_panic(self) {
        self.fail(IrohOperationErrorCode::Failed, INTERNAL_PANIC_MESSAGE);
    }
}

impl Drop for OperationStringCompletion {
    fn drop(&mut self) {
        if !self.done {
            self.done = true;
            (self.on_failure)(
                self.userdata as *mut c_void,
                IrohOperationErrorCode::NodeShutDown,
                c_string("node shut down"),
            );
        }
    }
}

/// Drop-guard completion for `IrohVoidOperationCallback`.
struct VoidOperationCompletion {
    userdata: usize,
    on_complete: extern "C" fn(userdata: *mut c_void),
    on_failure:
        extern "C" fn(userdata: *mut c_void, code: IrohOperationErrorCode, error: *const c_char),
    done: bool,
}

impl VoidOperationCompletion {
    fn succeed(mut self) {
        self.done = true;
        (self.on_complete)(self.userdata as *mut c_void);
    }

    fn fail(mut self, code: IrohOperationErrorCode, message: impl Into<String>) {
        let err = c_string(message.into());
        self.done = true;
        (self.on_failure)(self.userdata as *mut c_void, code, err);
    }
}

impl FailOnPanic for VoidOperationCompletion {
    fn fail_panic(self) {
        self.fail(IrohOperationErrorCode::Failed, INTERNAL_PANIC_MESSAGE);
    }
}

impl Drop for VoidOperationCompletion {
    fn drop(&mut self) {
        if !self.done {
            self.done = true;
            (self.on_failure)(
                self.userdata as *mut c_void,
                IrohOperationErrorCode::NodeShutDown,
                c_string("node shut down"),
            );
        }
    }
}

/// Drop-guard completion for `IrohGetAsyncCallback`.
struct GetAsyncCompletion {
    userdata: usize,
    on_progress: Option<extern "C" fn(userdata: *mut c_void, downloaded: u64, total: u64)>,
    on_success: extern "C" fn(userdata: *mut c_void, bytes: IrohOwnedBytes),
    on_failure: extern "C" fn(
        userdata: *mut c_void,
        code: IrohOperationErrorCode,
        size: u64,
        error: *const c_char,
    ),
    done: bool,
}

impl GetAsyncCompletion {
    fn progress(&self, downloaded: u64, total: u64) {
        if let Some(on_progress) = self.on_progress {
            on_progress(self.userdata as *mut c_void, downloaded, total);
        }
    }

    fn succeed(mut self, bytes: Vec<u8>) {
        let owned = into_owned_bytes(bytes);
        self.done = true;
        (self.on_success)(self.userdata as *mut c_void, owned);
    }

    fn fail(mut self, code: IrohOperationErrorCode, size: u64, message: impl Into<String>) {
        let err = c_string(message.into());
        self.done = true;
        (self.on_failure)(self.userdata as *mut c_void, code, size, err);
    }
}

impl FailOnPanic for GetAsyncCompletion {
    fn fail_panic(self) {
        self.fail(IrohOperationErrorCode::Failed, 0, INTERNAL_PANIC_MESSAGE);
    }
}

impl Drop for GetAsyncCompletion {
    fn drop(&mut self) {
        if !self.done {
            self.done = true;
            (self.on_failure)(
                self.userdata as *mut c_void,
                IrohOperationErrorCode::NodeShutDown,
                0,
                c_string("node shut down"),
            );
        }
    }
}

/// Drop-guard completion for `IrohNodeInfoAsyncCallback`.
struct NodeInfoAsyncCompletion {
    userdata: usize,
    on_success: extern "C" fn(userdata: *mut c_void, info: IrohNodeInfo),
    on_failure:
        extern "C" fn(userdata: *mut c_void, code: IrohOperationErrorCode, error: *const c_char),
    done: bool,
}

impl NodeInfoAsyncCompletion {
    fn succeed(mut self, info: crate::node::NodeInfo) {
        let info = ffi_node_info(info);
        self.done = true;
        (self.on_success)(self.userdata as *mut c_void, info);
    }

    fn fail(mut self, code: IrohOperationErrorCode, message: impl Into<String>) {
        let err = c_string(message.into());
        self.done = true;
        (self.on_failure)(self.userdata as *mut c_void, code, err);
    }
}

impl FailOnPanic for NodeInfoAsyncCompletion {
    fn fail_panic(self) {
        self.fail(IrohOperationErrorCode::Failed, INTERNAL_PANIC_MESSAGE);
    }
}

impl Drop for NodeInfoAsyncCompletion {
    fn drop(&mut self) {
        if !self.done {
            self.done = true;
            (self.on_failure)(
                self.userdata as *mut c_void,
                IrohOperationErrorCode::NodeShutDown,
                c_string("node shut down"),
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

/// Build a `CString` with interior NULs stripped (never panics on NUL).
fn to_c_string(s: impl Into<String>) -> CString {
    let s: String = s.into();
    let s = if s.contains('\0') {
        s.replace('\0', "")
    } else {
        s
    };
    CString::new(s).unwrap()
}

/// A C string for Swift to free with `iroh_string_free` (interior NULs are dropped).
fn c_string(s: impl Into<String>) -> *mut c_char {
    to_c_string(s).into_raw()
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

    #[test]
    fn test_spawn_thread_or_inline_runs_work() {
        let (tx, rx) = std::sync::mpsc::channel();
        spawn_thread_or_inline("iroh-test-worker", move || {
            let _ = tx.send(42u32);
        });
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
            42
        );
    }

    #[test]
    fn test_operation_error_code_distinguishes_timeout_from_transport_message() {
        use crate::node::OperationError;

        let transport_err = OperationError::Failed(anyhow::anyhow!("QUIC idle timed out"));
        assert_eq!(
            operation_error_code(&transport_err),
            (IrohOperationErrorCode::Failed, 0)
        );
        assert_eq!(
            operation_error_code(&OperationError::TimedOut),
            (IrohOperationErrorCode::TimedOut, 0)
        );
        assert_eq!(
            operation_error_code(&OperationError::SizeLimitExceeded(4096)),
            (IrohOperationErrorCode::SizeLimitExceeded, 4096)
        );
        assert_eq!(
            operation_error_code(&OperationError::Cancelled),
            (IrohOperationErrorCode::Cancelled, 0)
        );
        assert_eq!(
            operation_error_code(&OperationError::NodeShutDown),
            (IrohOperationErrorCode::NodeShutDown, 0)
        );
    }

    #[test]
    fn test_ffi_async_operations_and_error_codes() {
        use std::sync::mpsc;

        let dir = tempfile::tempdir().unwrap();
        let path_cstr = CString::new(dir.path().to_str().unwrap()).unwrap();
        let config = IrohNodeConfig {
            storage_path: path_cstr.as_ptr(),
            relay_enabled: false,
            custom_relay_url: std::ptr::null(),
            serves_blobs: false,
            accepts_pushes: false,
            dht_lookup: false,
            dht_publish: false,
        };

        // Pre-cancelled create returns Cancelled immediately.
        let cancel_tx_box = Box::into_raw(Box::new(mpsc::channel::<IrohOperationErrorCode>().0));
        let (cancel_tx, cancel_rx) = mpsc::channel::<IrohOperationErrorCode>();
        unsafe {
            *cancel_tx_box = cancel_tx;
        }
        extern "C" fn create_ok_unexpected(_: *mut c_void, _: *mut IrohNodeHandle) {
            panic!("should not succeed when pre-cancelled");
        }
        extern "C" fn create_fail_record(
            userdata: *mut c_void,
            code: IrohOperationErrorCode,
            err: *const c_char,
        ) {
            unsafe { iroh_string_free(err as *mut c_char) };
            let tx =
                unsafe { Box::from_raw(userdata as *mut mpsc::Sender<IrohOperationErrorCode>) };
            let _ = tx.send(code);
        }
        let pre_cancel = iroh_cancel_token_new();
        unsafe {
            iroh_cancel_token_cancel(pre_cancel);
            iroh_node_create_async(
                config,
                pre_cancel,
                IrohNodeCreateAsyncCallback {
                    userdata: cancel_tx_box as *mut c_void,
                    on_success: create_ok_unexpected,
                    on_failure: create_fail_record,
                },
            );
            iroh_cancel_token_free(pre_cancel);
        }
        assert_eq!(
            cancel_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
            IrohOperationErrorCode::Cancelled
        );

        // Normal async create on background thread.
        let (create_tx, create_rx) = mpsc::channel::<Result<usize, String>>();
        extern "C" fn create_ok(userdata: *mut c_void, handle: *mut IrohNodeHandle) {
            let tx = unsafe { Box::from_raw(userdata as *mut mpsc::Sender<Result<usize, String>>) };
            let _ = tx.send(Ok(handle as usize));
        }
        extern "C" fn create_err(
            userdata: *mut c_void,
            _code: IrohOperationErrorCode,
            err: *const c_char,
        ) {
            let msg = unsafe { CStr::from_ptr(err) }
                .to_string_lossy()
                .into_owned();
            unsafe { iroh_string_free(err as *mut c_char) };
            let tx = unsafe { Box::from_raw(userdata as *mut mpsc::Sender<Result<usize, String>>) };
            let _ = tx.send(Err(msg));
        }
        let config2 = IrohNodeConfig {
            storage_path: path_cstr.as_ptr(),
            relay_enabled: false,
            custom_relay_url: std::ptr::null(),
            serves_blobs: false,
            accepts_pushes: false,
            dht_lookup: false,
            dht_publish: false,
        };
        unsafe {
            iroh_node_create_async(
                config2,
                std::ptr::null(),
                IrohNodeCreateAsyncCallback {
                    userdata: Box::into_raw(Box::new(create_tx)) as *mut c_void,
                    on_success: create_ok,
                    on_failure: create_err,
                },
            );
        }
        let handle = create_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap() as *mut IrohNodeHandle;

        // Async put.
        let payload = b"hello async ffi";
        let (put_tx, put_rx) = mpsc::channel::<Result<String, IrohOperationErrorCode>>();
        extern "C" fn put_ok(userdata: *mut c_void, ticket: *const c_char) {
            let s = unsafe { CStr::from_ptr(ticket) }
                .to_string_lossy()
                .into_owned();
            unsafe { iroh_string_free(ticket as *mut c_char) };
            let tx = unsafe {
                Box::from_raw(userdata as *mut mpsc::Sender<Result<String, IrohOperationErrorCode>>)
            };
            let _ = tx.send(Ok(s));
        }
        extern "C" fn put_err(
            userdata: *mut c_void,
            code: IrohOperationErrorCode,
            err: *const c_char,
        ) {
            unsafe { iroh_string_free(err as *mut c_char) };
            let tx = unsafe {
                Box::from_raw(userdata as *mut mpsc::Sender<Result<String, IrohOperationErrorCode>>)
            };
            let _ = tx.send(Err(code));
        }
        unsafe {
            iroh_put_async(
                handle,
                IrohBytes {
                    data: payload.as_ptr(),
                    len: payload.len(),
                },
                0,
                std::ptr::null(),
                IrohOperationCallback {
                    userdata: Box::into_raw(Box::new(put_tx)) as *mut c_void,
                    on_success: put_ok,
                    on_failure: put_err,
                },
            );
        }
        let ticket = put_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        let ticket_cstr = CString::new(ticket).unwrap();

        // Async get with max_size smaller than payload -> SizeLimitExceeded.
        let (get_tx, get_rx) = mpsc::channel::<Result<Vec<u8>, (IrohOperationErrorCode, u64)>>();
        extern "C" fn get_ok(userdata: *mut c_void, bytes: IrohOwnedBytes) {
            let slice = unsafe { std::slice::from_raw_parts(bytes.data, bytes.len) }.to_vec();
            iroh_bytes_free(bytes);
            let tx = unsafe {
                Box::from_raw(
                    userdata as *mut mpsc::Sender<Result<Vec<u8>, (IrohOperationErrorCode, u64)>>,
                )
            };
            let _ = tx.send(Ok(slice));
        }
        extern "C" fn get_err(
            userdata: *mut c_void,
            code: IrohOperationErrorCode,
            size: u64,
            err: *const c_char,
        ) {
            unsafe { iroh_string_free(err as *mut c_char) };
            let tx = unsafe {
                Box::from_raw(
                    userdata as *mut mpsc::Sender<Result<Vec<u8>, (IrohOperationErrorCode, u64)>>,
                )
            };
            let _ = tx.send(Err((code, size)));
        }
        unsafe {
            iroh_get_async(
                handle,
                ticket_cstr.as_ptr(),
                0,
                4,
                std::ptr::null(),
                IrohGetAsyncCallback {
                    userdata: Box::into_raw(Box::new(get_tx)) as *mut c_void,
                    on_progress: None,
                    on_success: get_ok,
                    on_failure: get_err,
                },
            );
        }
        assert_eq!(
            get_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
            Err((
                IrohOperationErrorCode::SizeLimitExceeded,
                payload.len() as u64
            ))
        );

        // Close cleanly.
        let (close_tx, close_rx) = mpsc::channel::<Result<(), String>>();
        extern "C" fn close_ok(userdata: *mut c_void) {
            let tx = unsafe { Box::from_raw(userdata as *mut mpsc::Sender<Result<(), String>>) };
            let _ = tx.send(Ok(()));
        }
        extern "C" fn close_err(userdata: *mut c_void, err: *const c_char) {
            let msg = unsafe { CStr::from_ptr(err) }
                .to_string_lossy()
                .into_owned();
            unsafe { iroh_string_free(err as *mut c_char) };
            let tx = unsafe { Box::from_raw(userdata as *mut mpsc::Sender<Result<(), String>>) };
            let _ = tx.send(Err(msg));
        }
        iroh_node_close(
            handle,
            IrohCloseCallback {
                userdata: Box::into_raw(Box::new(close_tx)) as *mut c_void,
                on_complete: close_ok,
                on_failure: close_err,
            },
        );
        close_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
    }

    #[test]
    fn test_ffi_catch_or_catches_panics_and_returns_fallback() {
        assert_eq!(ffi_catch_or("test_ok", 0u32, || 7u32), 7u32);

        let ptr: *mut c_char = ffi_catch_or(
            "test_ptr_str_panic",
            std::ptr::null_mut(),
            || -> *mut c_char {
                panic!("boom str");
            },
        );
        assert!(ptr.is_null());

        let ok = ffi_catch_or("test_bool_string_panic", false, || -> bool {
            panic!("{}", String::from("boom owned"));
        });
        assert!(!ok);

        ffi_catch_or("test_void_non_string_panic", (), || {
            std::panic::panic_any(12345usize);
        });
    }

    #[test]
    fn test_ffi_catch_completion_reports_panic_once_and_never_double_fires() {
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Record {
            successes: AtomicUsize,
            failures: Mutex<Vec<(Option<IrohOperationErrorCode>, String)>>,
        }

        extern "C" fn on_str_ok(userdata: *mut c_void, val: *const c_char) {
            unsafe { iroh_string_free(val as *mut c_char) };
            let rec = unsafe { &*(userdata as *const Record) };
            rec.successes.fetch_add(1, Ordering::SeqCst);
        }

        extern "C" fn on_str_err(userdata: *mut c_void, err: *const c_char) {
            let msg = unsafe { CStr::from_ptr(err) }
                .to_string_lossy()
                .into_owned();
            unsafe { iroh_string_free(err as *mut c_char) };
            let rec = unsafe { &*(userdata as *const Record) };
            rec.failures.lock().unwrap().push((None, msg));
        }

        // 1. Panic before completing: on_failure fires once with fixed "internal panic"
        // (never leaking the raw panic payload across FFI).
        let rec = Record {
            successes: AtomicUsize::new(0),
            failures: Mutex::new(Vec::new()),
        };
        let cb = IrohCallback {
            userdata: &rec as *const Record as *mut c_void,
            on_success: on_str_ok,
            on_failure: on_str_err,
        };
        ffi_catch_completion("test_panic_before", StringCompletion::new(cb), |_slot| {
            panic!("sensitive /secret/path");
        });
        assert_eq!(rec.successes.load(Ordering::SeqCst), 0);
        let failures = rec.failures.lock().unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].1, "internal panic");
        drop(failures);

        // 2. Panic AFTER on_success already ran: callback is NOT invoked again.
        let rec2 = Record {
            successes: AtomicUsize::new(0),
            failures: Mutex::new(Vec::new()),
        };
        let cb2 = IrohCallback {
            userdata: &rec2 as *const Record as *mut c_void,
            on_success: on_str_ok,
            on_failure: on_str_err,
        };
        ffi_catch_completion("test_panic_after_ok", StringCompletion::new(cb2), |slot| {
            slot.finish(|c| c.succeed("ok".to_string()));
            panic!("post-callback panic");
        });
        assert_eq!(rec2.successes.load(Ordering::SeqCst), 1);
        assert!(rec2.failures.lock().unwrap().is_empty());

        // 3. Panic AFTER on_failure already ran: on_failure is NOT invoked again.
        let rec3 = Record {
            successes: AtomicUsize::new(0),
            failures: Mutex::new(Vec::new()),
        };
        let cb3 = IrohCallback {
            userdata: &rec3 as *const Record as *mut c_void,
            on_success: on_str_ok,
            on_failure: on_str_err,
        };
        ffi_catch_completion(
            "test_panic_after_fail",
            StringCompletion::new(cb3),
            |slot| {
                slot.finish(|c| c.fail("regular error"));
                panic!("post-failure panic");
            },
        );
        assert_eq!(rec3.successes.load(Ordering::SeqCst), 0);
        let failures3 = rec3.failures.lock().unwrap();
        assert_eq!(failures3.len(), 1);
        assert_eq!(failures3[0].1, "regular error");
        drop(failures3);

        // 4. Structured-error callback: panic reports IrohOperationErrorCode::Failed.
        extern "C" fn on_fetch_ok(userdata: *mut c_void, _size: u64) {
            let rec = unsafe { &*(userdata as *const Record) };
            rec.successes.fetch_add(1, Ordering::SeqCst);
        }
        extern "C" fn on_fetch_err(
            userdata: *mut c_void,
            code: IrohOperationErrorCode,
            _size: u64,
            err: *const c_char,
        ) {
            let msg = unsafe { CStr::from_ptr(err) }
                .to_string_lossy()
                .into_owned();
            unsafe { iroh_string_free(err as *mut c_char) };
            let rec = unsafe { &*(userdata as *const Record) };
            rec.failures.lock().unwrap().push((Some(code), msg));
        }

        let rec4 = Record {
            successes: AtomicUsize::new(0),
            failures: Mutex::new(Vec::new()),
        };
        let completion4 = FetchCompletion {
            userdata: &rec4 as *const Record as usize,
            on_progress: None,
            on_success: on_fetch_ok,
            on_failure: on_fetch_err,
            done: false,
        };
        ffi_catch_completion("test_fetch_panic", completion4, |_slot| {
            panic!("fetch exploded");
        });
        assert_eq!(rec4.successes.load(Ordering::SeqCst), 0);
        let failures4 = rec4.failures.lock().unwrap();
        assert_eq!(failures4.len(), 1);
        assert_eq!(failures4[0].0, Some(IrohOperationErrorCode::Failed));
        assert_eq!(failures4[0].1, "internal panic");
    }

    #[test]
    fn test_async_and_worker_panics_vs_unrelated_unwind_drop() {
        use std::sync::mpsc;

        extern "C" fn on_ok(_: *mut c_void, _: *const c_char) {
            panic!("unexpected success");
        }
        extern "C" fn on_err(
            userdata: *mut c_void,
            code: IrohOperationErrorCode,
            err: *const c_char,
        ) {
            let msg = unsafe { CStr::from_ptr(err) }
                .to_string_lossy()
                .into_owned();
            unsafe { iroh_string_free(err as *mut c_char) };
            let tx = unsafe {
                Box::from_raw(userdata as *mut mpsc::Sender<(IrohOperationErrorCode, String)>)
            };
            let _ = tx.send((code, msg));
        }

        // 1. Unrelated unwind dropping a guard still reports NodeShutDown (not internal panic).
        let (tx1, rx1) = mpsc::channel::<(IrohOperationErrorCode, String)>();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _c = OperationStringCompletion {
                userdata: Box::into_raw(Box::new(tx1)) as usize,
                on_success: on_ok,
                on_failure: on_err,
                done: false,
            };
            panic!("unrelated outer unwind");
        }));
        assert_eq!(
            rx1.recv().unwrap(),
            (
                IrohOperationErrorCode::NodeShutDown,
                "node shut down".to_string()
            )
        );

        // 2. Panic inside CatchUnwindFuture (spawned Tokio task) reports Failed + "internal panic".
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (tx2, rx2) = mpsc::channel::<(IrohOperationErrorCode, String)>();
        let c2 = OperationStringCompletion {
            userdata: Box::into_raw(Box::new(tx2)) as usize,
            on_success: on_ok,
            on_failure: on_err,
            done: false,
        };
        rt.block_on(async move {
            match CatchUnwindFuture(async {
                panic!("task boom");
            })
            .await
            {
                Ok(()) => c2.succeed("ok".to_string()),
                Err(payload) => {
                    log_panic("test_task", payload);
                    c2.fail_panic();
                }
            }
        });
        assert_eq!(
            rx2.recv().unwrap(),
            (IrohOperationErrorCode::Failed, "internal panic".to_string())
        );

        // 3. Panic inside spawn_thread_or_inline worker using ffi_catch_completion reports Failed + "internal panic".
        let (tx3, rx3) = mpsc::channel::<(IrohOperationErrorCode, String)>();
        let c3 = OperationStringCompletion {
            userdata: Box::into_raw(Box::new(tx3)) as usize,
            on_success: on_ok,
            on_failure: on_err,
            done: false,
        };
        spawn_thread_or_inline("iroh-test-panic-worker", move || {
            ffi_catch_completion("test_worker", c3, |_slot| {
                panic!("worker boom");
            });
        });
        assert_eq!(
            rx3.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
            (IrohOperationErrorCode::Failed, "internal panic".to_string())
        );
    }

    /// Extract the slice between a `{` and its matching `}`, ignoring braces
    /// inside string/char literals and `//` or `/* */` comments.
    fn extract_brace_body(s: &str, open_brace: usize) -> &str {
        let bytes = s.as_bytes();
        let mut i = open_brace;
        let mut depth = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                    i += 2;
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
                b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                    i += 2;
                    while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                        i += 1;
                    }
                    i += 2;
                }
                b'"' => {
                    i += 1;
                    while i < bytes.len() {
                        if bytes[i] == b'\\' {
                            i += 2;
                        } else if bytes[i] == b'"' {
                            i += 1;
                            break;
                        } else {
                            i += 1;
                        }
                    }
                }
                b'\'' => {
                    // Distinguish char literals ('x', '\n', '\0') from lifetimes ('a).
                    if i + 2 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 2;
                        while i < bytes.len() && bytes[i] != b'\'' {
                            i += 1;
                        }
                        i += 1;
                    } else if i + 2 < bytes.len() && bytes[i + 2] == b'\'' {
                        i += 3;
                    } else {
                        i += 1;
                    }
                }
                b'{' => {
                    depth += 1;
                    i += 1;
                }
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &s[open_brace + 1..i];
                    }
                    i += 1;
                }
                _ => {
                    i += 1;
                }
            }
        }
        panic!("unterminated brace block in ffi.rs");
    }

    /// Guardrail: every `pub [unsafe] extern "C" fn` export in `ffi.rs` must
    /// wrap its body in `ffi_catch_or` or `ffi_catch_completion` so no panic
    /// can unwind across the C ABI boundary.
    #[test]
    fn test_every_extern_c_export_catches_panics() {
        let src = include_str!("ffi.rs");
        let prod_src = src.split("\nmod tests").next().unwrap();
        let mut exports = Vec::new();
        let mut search_from = 0;

        while let Some(rel) = prod_src[search_from..].find("extern \"C\" fn ") {
            let idx = search_from + rel;
            let line_start = prod_src[..idx].rfind('\n').map_or(0, |n| n + 1);
            let prefix = prod_src[line_start..idx].trim();
            if prefix == "pub" || prefix == "pub unsafe" {
                let rest = &prod_src[idx + "extern \"C\" fn ".len()..];
                let name_end = rest.find('(').unwrap();
                let name = rest[..name_end].trim();
                let body_open = rest.find('{').unwrap();
                let body = extract_brace_body(rest, body_open);
                assert!(
                    body.contains("ffi_catch_or(") || body.contains("ffi_catch_completion("),
                    "extern \"C\" export `{name}` must wrap its body in `ffi_catch_or` or `ffi_catch_completion`"
                );
                exports.push(name.to_string());
            }
            search_from = idx + "extern \"C\" fn ".len();
        }

        assert!(
            exports.len() >= 32,
            "expected at least 32 extern \"C\" exports, found {}: {exports:?}",
            exports.len()
        );
    }
}
