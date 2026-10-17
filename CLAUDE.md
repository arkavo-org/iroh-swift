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

# Run all Swift tests (IROH_LOCAL_DEV=1 uses the local XCFramework)
IROH_LOCAL_DEV=1 swift test

# Rust checks (CI runs clippy with -D warnings)
cd rust && cargo check
cd rust && cargo test
cd rust && cargo fmt --check
cd rust && cargo clippy --all-targets -- -D warnings

# Regenerate C header from Rust (build-xcframework.sh also does this).
# The modulemap uses include/iroh_swift.h; include/iroh_swift_ffi.h is a tracked
# twin kept identical.
cd rust && cbindgen --config cbindgen.toml --crate iroh-swift-ffi --output ../include/iroh_swift.h
cp include/iroh_swift.h include/iroh_swift_ffi.h
```

## Architecture

Three-layer design with FFI boundary:

```
Swift Layer (IrohSwift)           Rust FFI Layer                Iroh Libraries
┌─────────────────────┐          ┌────────────────────┐        ┌────────────────────┐
│ IrohNode (actor)    │──C ABI──▶│ ffi.rs (callbacks) │───────▶│ iroh 1.3.0         │
│ IrohConfig          │          │ node.rs (IrohNode) │        │ iroh-blobs 0.103.1 │
│ IrohError           │          └────────────────────┘        │ tokio runtime      │
└─────────────────────┘                                        └────────────────────┘
```

**Key concepts:**
- Swift actor wraps opaque `IrohNodeHandle` pointer from Rust
- Rust functions use callbacks; Swift converts to async/await via `CheckedContinuation`
- Each IrohNode owns its own Tokio runtime
- Manual memory management at FFI boundary (`iroh_string_free()`, `iroh_bytes_free()`, `iroh_node_destroy()`)
- Most FFI calls `block_on` the node runtime on the calling thread. Long transfers
  (`iroh_fetch_from_node`, `iroh_push_file_to_node`) instead spawn on the runtime and
  return at once; Swift passes an `IrohCancelToken` (`CancelTokenBox`) and cancels it
  from `withTaskCancellationHandler`. The Rust side watches a `child_token()` of it, so
  a cancel is seen synchronously. A completion guard always calls exactly one of
  success/failure, reporting `NodeShutDown` if the node closes mid-operation.

## Blob transfer semantics

- `endpointID()` / `info().nodeId`: `EndpointId` `Display` = 64 lowercase hex of the
  Ed25519 public key. The key is random per `IrohNode` (not persisted); `sign(_:)`
  uses it (`SecretKey::sign`, verifies under `verify_strict`).
- `fetch(hash:fromNode:...)` drives `iroh_blobs::get::fsm` directly: the size header
  arrives before content, so `maxSize` is enforced before any body is read. Verified
  leaves are written to a hidden sibling temp file, fsynced, then renamed; nothing goes
  into the local store. Not found: an iroh-blobs FsStore provider resets the stream
  with `ERR_INTERNAL` (3) before the size header for an unknown hash (EOF also counts);
  `ERR_PERMISSION` (1, e.g. tdf-iroh-s3's moderation block) maps to `fetchFailed`.
- `IrohConfig.servesBlobs = false`: no blobs ALPN on the router and no provider event
  handler. Fetch and push still work (they only need the store and endpoint).
- Serving nodes refuse pushes unless `IrohConfig.acceptsPushes` (default `false`): the
  provider `EventMask` is `DEFAULT` (`push: Disabled`) or `push: Notify` when accepted.
  iroh-blobs 0.103.1+ applies each mask field to its own request type (before #270,
  `mask.get` governed pushes too). A refused push writes nothing, but the pusher is not
  reliably told: its client stops reading at once and never sees the `ERR_PERMISSION`
  reset. A tiny blob, written in one go, reports success; one still being written when
  the provider's STOP_SENDING arrives fails with "sending stopped by peer" (1 MiB
  already fails on localhost; a race for sizes in between). `acceptsPushes` without
  `servesBlobs` is rejected (`invalidConfiguration`; node creation fails on the Rust side).
  The provider event channel is always drained: the provider awaits each send.
- Pushes (Data and file) leave a tagged blob in the local store; file push imports
  with `ImportMode::Copy` (needs free space for a copy).

## Addressing a remote node

- `rust/src/resolve.rs`: `RemoteTarget::parse` validates the node id, relay URL and every
  direct address (`IP:port`, `[IPv6]:port` or `host:port`) synchronously, so argument errors
  still fail before anything is spawned. `HostResolver::resolve_target` then resolves
  hostnames (all A + AAAA records) inside the cancellable future, so cancel and shutdown also
  stop a slow lookup. Order per host: public DoH resolvers (Cloudflare, Quad9; DoH-only,
  `FallbackMode::Never`, 5 s), then only if they return no address the OS resolver
  (`tokio::net::lookup_host` = getaddrinfo: current config, scoped VPN resolvers, hosts file; 5 s;
  not iroh's `with_system_defaults()`, which caches the nameservers forever and misses scoped ones). Literal `IP:port` does no DNS.
  Hostnames that fail are ignored unless nothing else is left (no relay, no literal address),
  then the call fails with `Failed` (Swift `fetchFailed` / `pushFailed`), except with
  `dhtLookup` on (`HostResolverConfig.fall_through_to_lookup`, set by `IrohNode`): then it logs
  a warning and dials by endpoint ID alone so the DHT / n0 lookup can find the node during a DNS
  outage (only when the DHT actually started). No direct addresses at all is always left to
  address lookup. The resolver lives on `IrohNode` (`NodeOptions.host_resolver` overrides the DoH
  list and timeouts, Rust only; tests point it at `127.0.0.1:1`).
- Data pushes (`iroh_push_bytes_to_node`, used by both Swift `pushToNode(data:)` overloads) and file
  pushes spawn on the runtime with a cancel token, so a cancel also stops DNS and nothing is sent
  after it; `timeout_ms` (code `TimedOut`) covers DNS + import + transfer. The older blocking
  `iroh_push_to_node[_with_options]` exports remain for C callers but are not cancellable.
- `IrohConfig.dhtLookup` / `dhtPublish` -> `NodeOptions` -> `DhtAddressLookup` on the endpoint
  builder (`iroh-mainline-address-lookup`). Lookup alone uses `no_publish()`. `dhtPublish`
  requires `dhtLookup` and `servesBlobs` (Swift `validate()` and Rust both reject otherwise);
  published addresses are relay-only unless relays are disabled, then unfiltered.
  n0-mainline resolves its bootstrap hostnames with blocking DNS on the building thread, once,
  dropping failures, so `IrohNode::new_with_options` resolves `DEFAULT_DHT_BOOTSTRAP` itself
  (`HostResolver::resolve_bootstrap`, <= 8 s, IPv4 only) and passes IPs; none resolved = no DHT
  address lookup, a `tracing::warn!`, `NodeInfo.dht_active == false` (Swift `info().dhtActive`).
  The default list is a copy of n0-mainline's private constant: re-check it on upgrades.
  Caveats: `get_mutable_most_recent` returns the first record received (a higher `seq` arriving
  later is ignored); cancelling a fetch does not stop an in-flight DHT traversal.
  `NodeOptions.dht_bootstrap` swaps the bootstrap nodes: the Rust tests run a local
  `n0_mainline::Testnet` (a DHT cannot be exercised from Swift tests offline).

## Key Files

- `rust/src/ffi.rs` - C ABI exports: `iroh_node_create()`, `iroh_put()`, `iroh_get()`, `iroh_fetch_from_node()`, `iroh_push_file_to_node()`, cancel tokens
- `rust/src/resolve.rs` - `host:port` parsing, `HostResolver` (DoH then system DNS)
- `rust/src/node.rs` - Rust IrohNode managing Endpoint, FsStore, Router; `fetch_to_path`, `push_path`
- `Sources/IrohSwift/IrohNode.swift` - Swift actor with async/await API
- `Sources/IrohSwift/IrohNode+Fetch.swift`, `IrohNode+Push.swift`, `IrohNode+Identity.swift` - fetch-from-node, push, endpoint id / signing / BLAKE3
- `Sources/IrohSwift/FFISupport.swift` - `CancelTokenBox` and C-string helpers
- `scripts/build-xcframework.sh` - Builds static libs for all platforms into XCFramework
- `VERSION` - Single source of truth for version (sync with Cargo.toml and Package.swift)

## Version Management

Version must be synchronized across these files:
- `VERSION` (source of truth)
- `rust/Cargo.toml` (package.version)
- `rust/Cargo.lock` (run `cd rust && cargo check` after updating Cargo.toml; a bare `cargo update` would also move every dependency)
- `Package.swift` (both `version` string AND `checksum`, updated by CI on release)

**Known pitfall:** The `version` variable in `Package.swift` controls the download URL for the binary target. If it falls out of sync with `VERSION`, SPM will fetch the wrong artifact and the checksum will fail. The 0.3.1 release hit this — `VERSION` and `Cargo.toml` were bumped but `Package.swift` still pointed at 0.3.0. When bumping versions, always verify `Package.swift` version matches `VERSION`.

## Platform Targets

- iOS 26+ / macOS 26+
- XCFramework slices (keep all of them; consumers build `generic/platform=iOS Simulator` and `generic/platform=macOS` for both architectures):
  - `ios-arm64`: `aarch64-apple-ios`
  - `ios-arm64_x86_64-simulator`: `aarch64-apple-ios-sim` + `x86_64-apple-ios`, merged with `lipo`
  - `macos-arm64_x86_64`: `aarch64-apple-darwin` + `x86_64-apple-darwin`, merged with `lipo`
- Dropping an x86_64 slice forces every consumer to set `EXCLUDED_ARCHS = x86_64`; otherwise their link fails with `Undefined symbols for architecture x86_64`.

## Testing

Integration tests require a built XCFramework. Run `./scripts/build-xcframework.sh` first.

Tests cover: node creation, put/get with tickets, empty data handling, cross-node transfers.

Cross-node tests run two in-process nodes with relays disabled and connect over
`info().directAddrs`; without relay or direct addresses `connect` waits on discovery and
hangs. Give every node its own temp storage directory (a shared FsStore path deadlocks
on the redb lock) and `close()` nodes explicitly.

## Constraints

- No Ruby code (per user preference)
- Strict Swift 6.2 concurrency checking enabled
- Dual licensed: MIT + Apache 2.0
