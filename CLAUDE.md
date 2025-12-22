# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Swift bindings for Iroh blob storage, enabling iOS/macOS applications to perform distributed peer-to-peer data transfer. Uses a Rust FFI layer with callback-based async that bridges to Swift's async/await via continuations.

## Build Commands

```bash
# Build XCFramework (must run before Swift tests)
./scripts/build-xcframework.sh

# Build Swift package
swift build

# Run all Swift tests
swift test

# Rust checks
cd rust && cargo check
cd rust && cargo test
cd rust && cargo fmt --check
cd rust && cargo clippy

# Regenerate C header from Rust
cd rust && cbindgen --config cbindgen.toml --output ../include/iroh_swift_ffi.h
```

## Architecture

Three-layer design with FFI boundary:

```
Swift Layer (IrohSwift)           Rust FFI Layer                Iroh Libraries
┌─────────────────────┐          ┌────────────────────┐        ┌───────────────┐
│ IrohNode (actor)    │──C ABI──▶│ ffi.rs (callbacks) │──────▶│ iroh 0.95     │
│ IrohConfig          │          │ node.rs (IrohNode) │       │ iroh-blobs 0.97│
│ IrohError           │          └────────────────────┘        │ tokio runtime │
└─────────────────────┘                                        └───────────────┘
```

**Key concepts:**
- Swift actor wraps opaque `IrohNodeHandle` pointer from Rust
- Rust functions use callbacks; Swift converts to async/await via `CheckedContinuation`
- Each IrohNode owns its own Tokio runtime
- Manual memory management at FFI boundary (`iroh_string_free()`, `iroh_bytes_free()`, `iroh_node_destroy()`)

## Key Files

- `rust/src/ffi.rs` - C ABI exports: `iroh_node_create()`, `iroh_put()`, `iroh_get()`
- `rust/src/node.rs` - Rust IrohNode managing Endpoint, FsStore, Router
- `Sources/IrohSwift/IrohNode.swift` - Swift actor with async/await API
- `scripts/build-xcframework.sh` - Builds static libs for all platforms into XCFramework
- `VERSION` - Single source of truth for version (sync with Cargo.toml and Package.swift)

## Version Management

Version must be synchronized across these files:
- `VERSION` (source of truth)
- `rust/Cargo.toml` (package.version)
- `rust/Cargo.lock` (run `cd rust && cargo update` after updating Cargo.toml)
- `Package.swift` (binaryTarget checksum, updated by CI on release)

## Platform Targets

- iOS 26+ / macOS 26+
- ARM64 only: `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `aarch64-apple-darwin`

## Testing

Integration tests require a built XCFramework. Run `./scripts/build-xcframework.sh` first.

Tests cover: node creation, put/get with tickets, empty data handling, relay connectivity, cross-node transfers.

## Constraints

- No Ruby code (per user preference)
- Strict Swift 6.2 concurrency checking enabled
- Dual licensed: MIT + Apache 2.0
