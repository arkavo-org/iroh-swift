# Push-to-Node + 0.3.2 Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add push-to-node blob transfer support and bump to 0.3.2, resolving issues #10 and #11.

**Architecture:** New `push_to_node` method on IrohNode (Rust) that adds data to the local store, connects to a remote node via `EndpointAddr`, and pushes the blob using `Remote::execute_push()`. Wired through C FFI with callback pattern and exposed as Swift async/await via continuations. Optional `relay_url` parameter enables direct routing to known servers.

**Tech Stack:** Rust (iroh 0.97, iroh-blobs 0.99), C ABI (cbindgen), Swift 6.2 (actors, async/await)

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `rust/src/node.rs` | Modify | Add `push_to_node` and `push_to_node_with_timeout` methods |
| `rust/src/ffi.rs` | Modify | Add `iroh_push_to_node` and `iroh_push_to_node_with_options` FFI exports |
| `include/iroh_swift_ffi.h` | Regenerate | cbindgen adds new function declarations |
| `Sources/IrohSwift/IrohNode+Push.swift` | Create | Swift async API for push-to-node |
| `Sources/IrohSwift/IrohError.swift` | Modify | Add `pushFailed` error case |
| `Tests/IrohSwiftTests/PushToNodeTests.swift` | Create | Swift integration + error path tests |
| `VERSION` | Modify | `0.3.1` → `0.3.2` |
| `rust/Cargo.toml` | Modify | version bump |
| `Package.swift` | Modify | version string bump |

---

### Task 1: Version Bump

**Files:**
- Modify: `VERSION`
- Modify: `rust/Cargo.toml:3`
- Modify: `Package.swift:5`

- [ ] **Step 1: Bump VERSION file**

Change contents of `VERSION` from `0.3.1` to `0.3.2`:

```
0.3.2
```

- [ ] **Step 2: Bump rust/Cargo.toml**

In `rust/Cargo.toml`, change line 3:

```toml
version = "0.3.2"
```

- [ ] **Step 3: Update Cargo.lock**

Run: `cd rust && cargo update --workspace`
Expected: `Cargo.lock` updated with new version

- [ ] **Step 4: Bump Package.swift version string**

In `Package.swift`, change line 5:

```swift
let version = "0.3.2"
```

- [ ] **Step 5: Verify Rust builds**

Run: `cd rust && cargo check`
Expected: Compiles with no errors

- [ ] **Step 6: Commit**

```bash
git add VERSION rust/Cargo.toml rust/Cargo.lock Package.swift
git commit -m "chore: bump version to 0.3.2"
```

---

### Task 2: Rust Core — push_to_node on IrohNode

**Files:**
- Modify: `rust/src/node.rs`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block at the bottom of `rust/src/node.rs`:

```rust
#[test]
fn test_push_to_node_roundtrip() {
    let dir_a = tempdir().unwrap();
    let dir_b = tempdir().unwrap();

    // Create two nodes, relay disabled (local connection)
    let node_a = IrohNode::new(dir_a.path().to_path_buf(), false, None, false).unwrap();
    let node_b = IrohNode::new(dir_b.path().to_path_buf(), false, None, false).unwrap();

    // Get node B's ID
    let node_b_info = node_b.info().unwrap();
    let node_b_id = &node_b_info.node_id;

    // Push data from A to B
    let data = b"Push test data!";
    let hash_hex = node_a.push_to_node(node_b_id, None, data).unwrap();

    // Verify hash is valid hex (64 chars for blake3)
    assert_eq!(hash_hex.len(), 64);
    assert!(hash_hex.chars().all(|c| c.is_ascii_hexdigit()));

    node_a.shutdown().unwrap();
    node_b.shutdown().unwrap();
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd rust && cargo test test_push_to_node_roundtrip -- --nocapture`
Expected: FAIL with compile error — `push_to_node` method not found

- [ ] **Step 3: Add imports for push_to_node**

At the top of `rust/src/node.rs`, add to the existing imports:

```rust
use iroh::EndpointAddr;
use iroh_blobs::protocol::{GetRequest, PushRequest};
```

The existing import `use iroh_blobs::{ALPN as BLOBS_ALPN, ...}` already provides the ALPN constant.

- [ ] **Step 4: Implement push_to_node**

Add these methods to the `impl IrohNode` block in `rust/src/node.rs`, after the `get_with_timeout` method (after line 336) and before the `info` method:

```rust
    /// Push data to a remote node.
    ///
    /// Adds the data to the local store, connects to the remote node,
    /// and pushes the blob. Returns the blob hash as a hex string.
    ///
    /// # Arguments
    /// * `remote_node_id_hex` - The remote node's ID as a hex string
    /// * `relay_url` - Optional relay URL for routing the connection
    /// * `data` - The bytes to push
    pub fn push_to_node(
        &self,
        remote_node_id_hex: &str,
        relay_url: Option<&str>,
        data: &[u8],
    ) -> Result<String> {
        self.runtime.block_on(async {
            // Add bytes to local store first
            let tag = self
                .store
                .add_slice(data)
                .await
                .context("Failed to add bytes to store")?;

            // Parse remote node ID
            let node_id: iroh::EndpointId = remote_node_id_hex
                .parse()
                .context("Failed to parse remote node ID")?;

            // Build EndpointAddr with optional relay
            let endpoint_addr = if let Some(url) = relay_url {
                let relay_url: RelayUrl = url.parse().context("Invalid relay URL")?;
                let mut addr = EndpointAddr::from(node_id);
                addr.addrs.insert(iroh::TransportAddr::Relay(relay_url));
                addr
            } else {
                EndpointAddr::from(node_id)
            };

            // Connect to remote node
            let conn = self
                .endpoint
                .connect(endpoint_addr, BLOBS_ALPN)
                .await
                .context("Failed to connect to remote node")?;

            // Push the blob
            let request = PushRequest::from(GetRequest::blob(tag.hash));
            self.store
                .remote()
                .execute_push(conn, request)
                .await
                .context("Failed to push blob to remote node")?;

            Ok(hex::encode(tag.hash.as_bytes()))
        })
    }

    /// Push data to a remote node with an optional timeout.
    ///
    /// # Arguments
    /// * `remote_node_id_hex` - The remote node's ID as a hex string
    /// * `relay_url` - Optional relay URL for routing the connection
    /// * `data` - The bytes to push
    /// * `timeout_ms` - Timeout in milliseconds (0 = no timeout)
    pub fn push_to_node_with_timeout(
        &self,
        remote_node_id_hex: &str,
        relay_url: Option<&str>,
        data: &[u8],
        timeout_ms: u64,
    ) -> Result<String> {
        self.runtime.block_on(async {
            let fut = async {
                let tag = self
                    .store
                    .add_slice(data)
                    .await
                    .context("Failed to add bytes to store")?;

                let node_id: iroh::EndpointId = remote_node_id_hex
                    .parse()
                    .context("Failed to parse remote node ID")?;

                let endpoint_addr = if let Some(url) = relay_url {
                    let relay_url: RelayUrl = url.parse().context("Invalid relay URL")?;
                    let mut addr = EndpointAddr::from(node_id);
                    addr.addrs.insert(iroh::TransportAddr::Relay(relay_url));
                    addr
                } else {
                    EndpointAddr::from(node_id)
                };

                let conn = self
                    .endpoint
                    .connect(endpoint_addr, BLOBS_ALPN)
                    .await
                    .context("Failed to connect to remote node")?;

                let request = PushRequest::from(GetRequest::blob(tag.hash));
                self.store
                    .remote()
                    .execute_push(conn, request)
                    .await
                    .context("Failed to push blob to remote node")?;

                Ok::<_, anyhow::Error>(hex::encode(tag.hash.as_bytes()))
            };

            if timeout_ms == 0 {
                fut.await
            } else {
                tokio::time::timeout(Duration::from_millis(timeout_ms), fut)
                    .await
                    .context("Operation timed out")?
            }
        })
    }
```

- [ ] **Step 5: Run the test**

Run: `cd rust && cargo test test_push_to_node_roundtrip -- --nocapture`
Expected: PASS

- [ ] **Step 6: Run all existing tests to check for regressions**

Run: `cd rust && cargo test`
Expected: All tests pass

- [ ] **Step 7: Commit**

```bash
git add rust/src/node.rs
git commit -m "feat: add push_to_node to IrohNode (Rust core)"
```

---

### Task 3: Rust FFI — iroh_push_to_node exports

**Files:**
- Modify: `rust/src/ffi.rs`

- [ ] **Step 1: Add iroh_push_to_node FFI function**

Add the following after the `iroh_put_with_options` function (after line 858) in `rust/src/ffi.rs`:

```rust
/// Push data to a remote node by its node ID.
///
/// # Safety
/// - `handle` must be a valid node handle
/// - `remote_node_id_hex` must be a valid null-terminated UTF-8 hex string
/// - `relay_url` may be null; if non-null, must be a valid null-terminated UTF-8 string
/// - `bytes.data` must point to valid memory for `bytes.len` bytes
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_push_to_node(
    handle: *const IrohNodeHandle,
    remote_node_id_hex: *const c_char,
    relay_url: *const c_char,
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

    // Parse node ID string
    let node_id_str = match unsafe { CStr::from_ptr(remote_node_id_hex) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = CString::new(format!("Invalid node ID string: {}", e)).unwrap();
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
                let error = CString::new(format!("Invalid relay URL string: {}", e)).unwrap();
                (callback.on_failure)(callback.userdata, error.into_raw());
                return;
            }
        }
    };

    // Copy the bytes to own them (Swift memory may not be stable)
    let data = if bytes.data.is_null() || bytes.len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(bytes.data, bytes.len).to_vec() }
    };

    let node = unsafe { &*(handle as *const IrohNode) };

    match node.push_to_node(&node_id_str, relay_url_str.as_deref(), &data) {
        Ok(hash_hex) => {
            let hash_cstr = CString::new(hash_hex).unwrap();
            (callback.on_success)(callback.userdata, hash_cstr.into_raw());
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}

/// Push data to a remote node with options (e.g., timeout).
///
/// # Safety
/// - `handle` must be a valid node handle
/// - `remote_node_id_hex` must be a valid null-terminated UTF-8 hex string
/// - `relay_url` may be null; if non-null, must be a valid null-terminated UTF-8 string
/// - `bytes.data` must point to valid memory for `bytes.len` bytes
/// - `callback` must have valid function pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iroh_push_to_node_with_options(
    handle: *const IrohNodeHandle,
    remote_node_id_hex: *const c_char,
    relay_url: *const c_char,
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

    let node_id_str = match unsafe { CStr::from_ptr(remote_node_id_hex) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            let error = CString::new(format!("Invalid node ID string: {}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
            return;
        }
    };

    let relay_url_str = if relay_url.is_null() {
        None
    } else {
        match unsafe { CStr::from_ptr(relay_url) }.to_str() {
            Ok(s) => Some(s.to_string()),
            Err(e) => {
                let error = CString::new(format!("Invalid relay URL string: {}", e)).unwrap();
                (callback.on_failure)(callback.userdata, error.into_raw());
                return;
            }
        }
    };

    let data = if bytes.data.is_null() || bytes.len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(bytes.data, bytes.len).to_vec() }
    };

    let node = unsafe { &*(handle as *const IrohNode) };
    let timeout_ms = options.timeout_ms;

    match node.push_to_node_with_timeout(&node_id_str, relay_url_str.as_deref(), &data, timeout_ms) {
        Ok(hash_hex) => {
            let hash_cstr = CString::new(hash_hex).unwrap();
            (callback.on_success)(callback.userdata, hash_cstr.into_raw());
        }
        Err(e) => {
            let error = CString::new(format!("{:#}", e)).unwrap();
            (callback.on_failure)(callback.userdata, error.into_raw());
        }
    }
}
```

- [ ] **Step 2: Verify Rust compiles**

Run: `cd rust && cargo check`
Expected: Compiles with no errors

- [ ] **Step 3: Run all Rust tests**

Run: `cd rust && cargo test`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add rust/src/ffi.rs
git commit -m "feat: add iroh_push_to_node FFI exports"
```

---

### Task 4: Regenerate C Header

**Files:**
- Regenerate: `include/iroh_swift_ffi.h`

- [ ] **Step 1: Regenerate the header**

Run: `cd rust && cbindgen --config cbindgen.toml --output ../include/iroh_swift_ffi.h`
Expected: Header regenerated with `iroh_push_to_node` and `iroh_push_to_node_with_options` declarations

- [ ] **Step 2: Verify new functions appear in header**

Run: `grep "iroh_push_to_node" include/iroh_swift_ffi.h`
Expected: Two function declarations visible

- [ ] **Step 3: Commit**

```bash
git add include/iroh_swift_ffi.h
git commit -m "chore: regenerate C header with push-to-node functions"
```

---

### Task 5: Swift Error Case

**Files:**
- Modify: `Sources/IrohSwift/IrohError.swift`

- [ ] **Step 1: Add pushFailed error case**

In `Sources/IrohSwift/IrohError.swift`, add after the `getFailed` case (after line 10):

```swift
    /// Failed to push data to a remote node.
    case pushFailed(String)
```

- [ ] **Step 2: Add error description**

In the `errorDescription` computed property, add after the `getFailed` case (after line 75):

```swift
        case .pushFailed(let msg):
            return "Failed to push data: \(msg)"
```

- [ ] **Step 3: Commit**

```bash
git add Sources/IrohSwift/IrohError.swift
git commit -m "feat: add pushFailed error case"
```

---

### Task 6: Swift Push API

**Files:**
- Create: `Sources/IrohSwift/IrohNode+Push.swift`

- [ ] **Step 1: Create IrohNode+Push.swift**

Create `Sources/IrohSwift/IrohNode+Push.swift`:

```swift
import Foundation
import IrohSwiftFFI

extension IrohNode {
    /// Push data to a remote node by its node ID.
    ///
    /// Adds the data to the local store, connects to the remote node,
    /// and pushes the blob.
    ///
    /// - Parameters:
    ///   - nodeId: The remote node's ID as a hex string.
    ///   - relayUrl: Optional relay URL for routing the connection.
    ///   - data: The data to push.
    /// - Returns: The blob hash as a hex string.
    /// - Throws: `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.pushFailed` if the operation fails,
    ///           `CancellationError` if the task was cancelled.
    public func pushToNode(nodeId: String, relayUrl: String? = nil, data: Data) async throws -> String {
        try ensureNotClosed()
        try Task.checkCancellation()
        return try await withCheckedThrowingContinuation { continuation in
            nodeId.withCString { nodeIdPtr in
                data.withUnsafeBytes { buffer in
                    let bytes = IrohBytes(
                        data: buffer.baseAddress?.assumingMemoryBound(to: UInt8.self),
                        len: UInt(buffer.count)
                    )

                    let box = Unmanaged.passRetained(
                        ContinuationBox<String>(continuation)
                    ).toOpaque()

                    let callback = IrohCallback(
                        userdata: box,
                        on_success: { userdata, hashPtr in
                            let box = Unmanaged<ContinuationBox<String>>
                                .fromOpaque(userdata!)
                                .takeRetainedValue()
                            let hash = String(cString: hashPtr!)
                            iroh_string_free(UnsafeMutablePointer(mutating: hashPtr))
                            box.continuation.resume(returning: hash)
                        },
                        on_failure: { userdata, errorPtr in
                            let box = Unmanaged<ContinuationBox<String>>
                                .fromOpaque(userdata!)
                                .takeRetainedValue()
                            let message = String(cString: errorPtr!)
                            iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                            box.continuation.resume(throwing: IrohError.pushFailed(message))
                        }
                    )

                    if let relayUrl = relayUrl {
                        relayUrl.withCString { relayUrlPtr in
                            iroh_push_to_node(handle.pointer, nodeIdPtr, relayUrlPtr, bytes, callback)
                        }
                    } else {
                        iroh_push_to_node(handle.pointer, nodeIdPtr, nil, bytes, callback)
                    }
                }
            }
        }
    }

    /// Push data to a remote node with options.
    ///
    /// - Parameters:
    ///   - nodeId: The remote node's ID as a hex string.
    ///   - relayUrl: Optional relay URL for routing the connection.
    ///   - data: The data to push.
    ///   - options: Operation options including timeout.
    /// - Returns: The blob hash as a hex string.
    /// - Throws: `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.timeout` if the operation times out,
    ///           `IrohError.pushFailed` if the operation fails.
    public func pushToNode(nodeId: String, relayUrl: String? = nil, data: Data, options: OperationOptions) async throws -> String {
        try ensureNotClosed()
        try Task.checkCancellation()
        return try await withCheckedThrowingContinuation { continuation in
            nodeId.withCString { nodeIdPtr in
                data.withUnsafeBytes { buffer in
                    let bytes = IrohBytes(
                        data: buffer.baseAddress?.assumingMemoryBound(to: UInt8.self),
                        len: UInt(buffer.count)
                    )

                    let ffiOptions = IrohOperationOptions(timeout_ms: options.timeoutMs)

                    let box = Unmanaged.passRetained(
                        ContinuationBox<String>(continuation)
                    ).toOpaque()

                    let callback = IrohCallback(
                        userdata: box,
                        on_success: { userdata, hashPtr in
                            let box = Unmanaged<ContinuationBox<String>>
                                .fromOpaque(userdata!)
                                .takeRetainedValue()
                            let hash = String(cString: hashPtr!)
                            iroh_string_free(UnsafeMutablePointer(mutating: hashPtr))
                            box.continuation.resume(returning: hash)
                        },
                        on_failure: { userdata, errorPtr in
                            let box = Unmanaged<ContinuationBox<String>>
                                .fromOpaque(userdata!)
                                .takeRetainedValue()
                            let message = String(cString: errorPtr!)
                            iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                            if message.contains("timed out") {
                                box.continuation.resume(throwing: IrohError.timeout)
                            } else {
                                box.continuation.resume(throwing: IrohError.pushFailed(message))
                            }
                        }
                    )

                    if let relayUrl = relayUrl {
                        relayUrl.withCString { relayUrlPtr in
                            iroh_push_to_node_with_options(handle.pointer, nodeIdPtr, relayUrlPtr, bytes, ffiOptions, callback)
                        }
                    } else {
                        iroh_push_to_node_with_options(handle.pointer, nodeIdPtr, nil, bytes, ffiOptions, callback)
                    }
                }
            }
        }
    }
}
```

Note: `ContinuationBox` is defined as `private` in `IrohNode.swift`. Since Swift extensions in the same module can access `private` types defined in the same file — but NOT from a different file — we need to change `ContinuationBox` from `private` to `internal` (package-level). In `Sources/IrohSwift/IrohNode.swift`, change line 365:

```swift
final class ContinuationBox<T>: @unchecked Sendable {
```

(Remove the `private` keyword — `internal` is the default.)

- [ ] **Step 2: Verify Swift builds**

Run: `swift build` (requires XCFramework — if not available locally, verify with `IROH_LOCAL_DEV=1 swift build` or rebuild the XCFramework first)
Expected: Compiles with no errors

- [ ] **Step 3: Commit**

```bash
git add Sources/IrohSwift/IrohNode+Push.swift Sources/IrohSwift/IrohNode.swift
git commit -m "feat: add pushToNode Swift async API"
```

---

### Task 7: Swift Tests

**Files:**
- Create: `Tests/IrohSwiftTests/PushToNodeTests.swift`

- [ ] **Step 1: Create PushToNodeTests.swift**

Create `Tests/IrohSwiftTests/PushToNodeTests.swift`:

```swift
import Testing
import Foundation
@testable import IrohSwift

struct PushToNodeTests {
    @Test("pushToNode returns valid hash hex string")
    func testPushToNodeReturnsHash() async throws {
        // Create two nodes
        let configA = IrohConfig(relayEnabled: false)
        let configB = IrohConfig(relayEnabled: false)
        let nodeA = try await IrohNode(config: configA)
        let nodeB = try await IrohNode(config: configB)

        // Get node B's ID
        let nodeInfoB = try await nodeB.info()
        let nodeBId = nodeInfoB.nodeId

        // Push data from A to B
        let data = "Push test from Swift!".data(using: .utf8)!
        let hashHex = try await nodeA.pushToNode(nodeId: nodeBId, data: data)

        // Verify hash is 64-char hex
        #expect(hashHex.count == 64)
        #expect(hashHex.allSatisfy { $0.isHexDigit })

        try await nodeA.close()
        try await nodeB.close()
    }

    @Test("pushToNode with invalid node ID throws pushFailed")
    func testPushToNodeInvalidNodeId() async throws {
        let config = IrohConfig(relayEnabled: false)
        let node = try await IrohNode(config: config)

        let data = "test".data(using: .utf8)!

        await #expect(throws: IrohError.self) {
            try await node.pushToNode(nodeId: "not-a-valid-hex-node-id", data: data)
        }

        try await node.close()
    }

    @Test("pushToNode on closed node throws nodeClosed")
    func testPushToNodeClosedNode() async throws {
        let config = IrohConfig(relayEnabled: false)
        let node = try await IrohNode(config: config)
        try await node.close()

        let data = "test".data(using: .utf8)!

        await #expect(throws: IrohError.self) {
            try await node.pushToNode(nodeId: "deadbeef", data: data)
        }
    }
}
```

- [ ] **Step 2: Verify tests compile and run**

Run: `swift test --filter PushToNodeTests` (requires built XCFramework)
Expected: Tests pass (or compile correctly if XCFramework is only available via CI)

- [ ] **Step 3: Commit**

```bash
git add Tests/IrohSwiftTests/PushToNodeTests.swift
git commit -m "test: add push-to-node Swift tests"
```

---

### Task 8: Rust Lint and Format Check

**Files:** None (validation only)

- [ ] **Step 1: Run cargo fmt**

Run: `cd rust && cargo fmt`
Expected: Code formatted

- [ ] **Step 2: Run cargo clippy**

Run: `cd rust && cargo clippy -- -D warnings`
Expected: No warnings

- [ ] **Step 3: Run full test suite**

Run: `cd rust && cargo test`
Expected: All tests pass

- [ ] **Step 4: Commit any formatting changes**

```bash
git add -A rust/
git commit -m "style: apply cargo fmt"
```

(Skip if no changes.)
