//! Minimal FFI bindings for Iroh blob operations.
//!
//! This crate provides a C ABI interface to Iroh's blob storage,
//! exposing only the operations needed for Arkavo profiles:
//! - `put(bytes) -> ticket`
//! - `get(ticket) -> bytes`
//! - Node lifecycle management

mod ffi;
mod node;

pub use ffi::*;
