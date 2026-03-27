# 0.3.2 Release: Push-to-Node + Checksum Fix

## Issues

- **#10** — 0.3.1 release: xcframework artifact checksum mismatch
- **#11** — Add push-to-node support for blob transfers

## Summary

Release 0.3.2 adds push-to-node blob transfer support and supersedes the broken 0.3.1 SPM artifact, resolving both issues in a single release.

## Architecture

Push-to-node mirrors the existing pull (`get`) flow but in reverse direction. The data flow:

```
Swift: pushToNode(nodeId:data:) or pushToNode(nodeId:relayUrl:data:)
  → FFI: iroh_push_to_node(handle, node_id_hex, relay_url?, data, callback)
    → Rust: IrohNode::push_to_node(&self, node_id_hex, relay_url, data)
      1. store.add_slice(data)                → hash
      2. Build EndpointAddr from node_id + optional relay_url
      3. endpoint.connect(endpoint_addr, ALPN) → conn
      4. store.remote().execute_push(conn, PushRequest::from(GetRequest::blob(hash)))
      5. Return hash hex string
```

The underlying iroh-blobs 0.99 API already supports push via `Remote::execute_push()`. This design wires it through the FFI boundary and Swift layer using the same callback-to-continuation pattern as `put` and `get`.

**Addressing:** `EndpointAddr` contains a required `EndpointId` and an optional set of `TransportAddr` (relay URLs, IP addresses). When a `relay_url` is provided, an `EndpointAddr` is constructed with the relay for direct routing to a known server (e.g., `tdf-iroh-s3`). When omitted (`NULL` in FFI, `nil` in Swift), just the `EndpointId` is used, which relies on relay discovery or direct connection.

**Return value:** The blob hash (hex string), not a ticket. The caller already knows the remote node ID, so a ticket (which encodes source node address) is not useful.

## API Surface

### Rust Core (`rust/src/node.rs`)

```rust
/// Push data to a remote node.
///
/// Adds the data to the local store, connects to the remote node,
/// and pushes the blob. Returns the blob hash as a hex string.
///
/// `relay_url` is optional — when provided, the connection routes via that relay.
/// When `None`, relies on discovery or direct connection.
pub fn push_to_node(
    &self,
    remote_node_id_hex: &str,
    relay_url: Option<&str>,
    data: &[u8],
) -> Result<String>

/// Push data to a remote node with a timeout.
pub fn push_to_node_with_timeout(
    &self,
    remote_node_id_hex: &str,
    relay_url: Option<&str>,
    data: &[u8],
    timeout_ms: u64,
) -> Result<String>
```

Implementation details:
- Parse `remote_node_id_hex` as `EndpointId` (type alias for `PublicKey`, implements `FromStr`)
- If `relay_url` is provided, build `EndpointAddr { id, addrs: {Relay(url)} }`; otherwise use `EndpointId` alone (converts to `EndpointAddr` with empty addrs)
- `endpoint.connect(endpoint_addr, iroh_blobs::ALPN)` — accepts `impl Into<EndpointAddr>`
- `store.remote().execute_push(conn, PushRequest::from(GetRequest::blob(hash))).complete().await`

### Rust FFI (`rust/src/ffi.rs`)

```rust
/// Push data to a remote node by node ID.
/// `relay_url` is optional (may be NULL) — provides routing hint.
/// Callback receives the blob hash hex string on success.
#[unsafe(no_mangle)]
pub extern "C" fn iroh_push_to_node(
    handle: *mut IrohNodeHandle,
    remote_node_id_hex: *const c_char,
    relay_url: *const c_char,  // nullable
    data: IrohBytes,
    callback: IrohCallback,
)

/// Push data to a remote node with options (timeout).
#[unsafe(no_mangle)]
pub extern "C" fn iroh_push_to_node_with_options(
    handle: *mut IrohNodeHandle,
    remote_node_id_hex: *const c_char,
    relay_url: *const c_char,  // nullable
    data: IrohBytes,
    options: IrohOperationOptions,
    callback: IrohCallback,
)
```

Reuses existing `IrohCallback` (on_success with result string, on_failure with error string). No new callback types needed. Follows the same `spawn_blocking` + `block_on` pattern as `iroh_put`.

### C Header (`include/iroh_swift_ffi.h`)

Regenerated via `cbindgen`. Two new function declarations matching the FFI exports above.

### Swift (`Sources/IrohSwift/IrohNode+Push.swift`)

New extension file following the existing pattern (e.g., `IrohNode+Blobs.swift`):

```swift
extension IrohNode {
    /// Push data to a remote node by its node ID.
    /// `relayUrl` is optional — provides a routing hint for connecting to the remote node.
    /// Returns the blob hash as a hex string.
    public func pushToNode(nodeId: String, relayUrl: String? = nil, data: Data) async throws -> String

    /// Push data to a remote node with options.
    public func pushToNode(nodeId: String, relayUrl: String? = nil, data: Data, options: OperationOptions) async throws -> String
}
```

Uses the same `ContinuationBox` + `withCheckedThrowingContinuation` pattern as `put()` and `get()`.

## Version Bump

`0.3.1` → `0.3.2` in:

| File | Field |
|------|-------|
| `VERSION` | file content |
| `rust/Cargo.toml` | `package.version` |
| `rust/Cargo.lock` | via `cargo update` |
| `Package.swift` | `version` string (checksum set by CI) |

This naturally resolves #10: 0.3.2 produces a fresh artifact with a correct checksum. Consumers upgrade from 0.3.0 (current workaround) to 0.3.2.

## Files Changed

| File | Change |
|------|--------|
| `VERSION` | `0.3.1` → `0.3.2` |
| `rust/Cargo.toml` | version bump |
| `rust/Cargo.lock` | updated via cargo |
| `rust/src/node.rs` | `push_to_node`, `push_to_node_with_timeout` methods |
| `rust/src/ffi.rs` | `iroh_push_to_node`, `iroh_push_to_node_with_options` exports |
| `include/iroh_swift_ffi.h` | regenerated with new function declarations |
| `Sources/IrohSwift/IrohNode+Push.swift` | new extension file with Swift async API |
| `Tests/IrohSwiftTests/` | push-to-node tests |
| `Package.swift` | version string to `0.3.2` |

## Testing

### Rust unit test (`rust/src/node.rs`)

`test_push_to_node_roundtrip`:
1. Create two IrohNodes with relay disabled, separate temp directories
2. Node A calls `push_to_node(node_b_id, data)`
3. Verify Node B's store contains the blob by reading it via hash

### Swift integration test

`testPushToNode`:
1. Create two IrohNode instances
2. Node A pushes data to Node B via `pushToNode(nodeId:data:)`
3. Verify the returned hash is non-empty and valid hex

### Error path test

`testPushToNodeInvalidNodeId`:
- Push to an invalid node ID hex string returns a clear `IrohError`

## Motivation

The `tdf-iroh-s3` storage node accepts blobs via the iroh-blobs push protocol. Mobile clients (ClosureKB) need to push encrypted TDF files to this node. Without push support, clients must use ticket-based pull which requires the mobile node to be reachable via relay and a separate notification mechanism. Push eliminates both requirements.
