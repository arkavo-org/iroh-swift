# iroh-swift

Swift bindings for [Iroh](https://iroh.computer) - peer-to-peer blob storage and transfer.

## Features

- **Blob Storage**: Store and retrieve content-addressed data
- **Relay Support**: NAT traversal via n0 public relays

## Requirements

- Swift 6.2+
- iOS 26+ / macOS 26+
- Xcode 26+

## Installation

### Swift Package Manager

Add to your `Package.swift`:

```swift
dependencies: [
    .package(url: "https://github.com/arkavo-org/iroh-swift.git", from: "0.6.0")
]
```

Then add `IrohSwift` to your target dependencies:

```swift
.target(
    name: "YourApp",
    dependencies: ["IrohSwift"]
)
```

## Usage

### Blob Storage

```swift
import IrohSwift

// Create a node
let node = try await IrohNode()

// Store data and get a shareable ticket
let data = "Hello, Iroh!".data(using: .utf8)!
let ticket = try await node.put(data)

// Retrieve data using a ticket
let retrieved = try await node.get(ticket: ticket)
```

### Fetch a Blob from a Pinned Node

Download by BLAKE3 hash from one known node (no ticket) straight into a file.
Every byte is verified against the hash as it arrives; the file only appears at
`destination` once the whole blob is verified.

```swift
// A client-only node: it can fetch and push, but never serves blobs to others.
let viewer = try await IrohNode(config: IrohConfig(servesBlobs: false))

let task = Task {
    try await viewer.fetch(
        hash: blobHash,                 // 64 hex chars
        fromNode: serverNodeId,         // 64 hex chars, the only node contacted
        relayUrl: serverRelayUrl,       // optional address hints
        directAddrs: serverAddrs,
        to: cacheURL,
        maxSize: 512 * 1024 * 1024,     // checked before any content is read
        progress: { downloaded, total in /* cumulative bytes */ }
    )
}
// task.cancel() aborts the transfer and throws CancellationError.
```

`directAddrs` entries are `"ip:port"` (`"[ipv6]:port"` for IPv6), used as is, or
`"host:port"`, for example `"iroh.arkavo.net:11204"` (see
[Hostnames in direct addresses](#hostnames-in-direct-addresses)). With neither
`directAddrs` nor `relayUrl` the node is looked up by its ID (see
[Find a node by ID on the DHT](#find-a-node-by-id-on-the-dht)).

Errors: `IrohError.notFound` (the node does not have the blob),
`IrohError.sizeLimitExceeded(size)`, `CancellationError`, `IrohError.nodeClosed`,
and `IrohError.fetchFailed` for everything else. On any error no file is left at
`destination`.

### Sign with the Endpoint Key, then Push

```swift
let publisher = try await IrohNode()
let endpointID = try await publisher.endpointID()   // Ed25519 public key, 64 lowercase hex

let message = Data("""
    tdf-iroh-s3 publish-session v1
    \(endpointID)
    \(IrohBLAKE3.hex(Data(bearerToken.utf8)))
    \(timestamp)
    """.utf8)
let signature = try await publisher.sign(message)   // 64-byte Ed25519 signature

// Push from the same node: the endpoint key is per node instance, not persisted.
let hash = try await publisher.pushToNode(
    nodeId: serverNodeId,
    relayUrl: serverRelayUrl,
    fileURL: tdfFileURL                             // streamed, never loaded into memory
)
```

### Hostnames in direct addresses

Any `directAddrs` entry that is not a literal `ip:port` must be `host:port` (port
1 to 65535; IPv6 literals go in brackets: `[2001:db8::1]:11204`). The host is
resolved to all its A and AAAA records and every `IP:port` is dialled; literal
`ip:port` entries do no DNS at all. This applies to `fetch(...)` and every
`pushToNode(...)`.

Resolution order, per host: the public DNS-over-HTTPS resolvers first
(Cloudflare 1.1.1.1 / 1.0.0.1 and Quad9 9.9.9.9, up to 5 s), and only if none of
them returns an address, the operating system's resolver (`getaddrinfo`, up to 5 s;
it always uses the current network configuration, scoped VPN / split-DNS resolvers
and the hosts file, so `localhost` and enterprise names work). Each node builds
the DoH resolver once and reuses it; the OS stage is not cached by us. Hostnames must be ASCII (use punycode for
internationalized names).

A hostname cannot impersonate a node: you pin the endpoint ID and the QUIC
handshake checks it, so a wrong or poisoned DNS answer only makes the connection
fail. That makes a CNAME that follows a redeploy, such as `iroh.arkavo.net`, safe
to use.

Failure: a malformed entry (`host-without-port`, `host:0`, an unbracketed IPv6
address) fails the call before anything is sent. If hostnames were given, none
resolved, and there is no `relayUrl` and no literal address left, the call fails
with a message naming the hosts: `IrohError.fetchFailed` for `fetch`,
`IrohError.pushFailed` for `pushToNode`. **Unless the node was created with
`dhtLookup: true`:** then the call does not fail but carries on with the endpoint
ID alone, exactly like a call with no addresses, so the DHT (and n0's lookups) can
still find the node when DNS is down (a warning is logged). A hostname that fails
is also ignored when a relay URL or another address is available. Cancelling the
task also cancels a slow lookup.

### Find a Node by ID on the DHT

Opt in with `IrohConfig.dhtLookup` to resolve endpoint IDs on the BitTorrent
Mainline DHT (pkarr, via `iroh-mainline-address-lookup`), in addition to the
default n0 lookups. A node you only know by its ID can then be reached with no
relay URL and no addresses:

```swift
let viewer = try await IrohNode(config: IrohConfig(servesBlobs: false, dhtLookup: true))
try await viewer.fetch(hash: blobHash, fromNode: serverNodeId, to: cacheURL, maxSize: limit)
```

`dhtLookup` only reads from the DHT; a node never publishes unless you also set
`dhtPublish: true`, which requires `dhtLookup` and `servesBlobs` (a client-only
node is never dialled, so `IrohConfig.validate()` rejects `dhtPublish` on one with
`IrohError.invalidConfiguration`). A publishing node signs its address with its
endpoint key and republishes hourly. While relays are enabled only the relay URL
is published, not IP addresses, because the DHT is public; with
`relayEnabled: false` the direct addresses are published, because a peer could
use nothing else. Explicit `relayUrl` / `directAddrs` still apply alongside the
lookup. The DHT client opens its own UDP socket and contacts public bootstrap
nodes, which is why both flags default to `false`. Note that the endpoint key is
random per node instance (not persisted), so a publishing node's ID changes
whenever it is recreated.

At creation the node resolves the DHT bootstrap hostnames itself (DoH first, then
the OS, all names in parallel, at most about 8 seconds in total), so node creation
never blocks on unbounded DNS. If none resolves (started offline), the node starts
*without* DHT lookup, logs a warning, and reports `info().dhtActive == false`; it
does not retry, so recreate the node once the network is up. Two library caveats:
`n0-mainline` 0.7.1's `get_mutable_most_recent` keeps the first record any replica
answers with (a later, higher-`seq` record is ignored), so right after a node's
address changes a stale replica can win; and cancelling a fetch does not stop an
in-flight DHT traversal, which runs to its own bound (harmless, but not free).

### Receive Pushes

A serving node refuses pushes unless it opts in, because accepting them lets any
peer that can reach the node write to its disk:

```swift
let inbox = try await IrohNode(config: IrohConfig(servesBlobs: true, acceptsPushes: true))
```

`acceptsPushes` without `servesBlobs` throws `IrohError.invalidConfiguration`.
A node that refuses a push stores nothing, but the pusher is not reliably told:
a small blob can still return its hash, while a larger one fails with
`IrohError.pushFailed`. Confirm delivery out of band (for example by fetching
the hash back) when it matters.

### Blob Pinning

```swift
// After downloading content, pin it to prevent garbage collection
let ticketInfo = await validateTicket(ticket)
try await node.tagBlob(hash: ticketInfo.hash!, name: "pins/my-content")

// Create a new ticket pointing to this node
let bootstrapTicket = try await node.createTicket(hash: ticketInfo.hash!)

// Remove the pin when no longer needed
try await node.untagBlob(name: "pins/my-content")
```

## API Reference

### IrohNode

| Method | Description |
|--------|-------------|
| `init(config:)` | Create a node with optional configuration |
| `put(_:)` | Store data, return shareable ticket |
| `get(ticket:)` | Download data using a ticket |
| `fetch(hash:fromNode:relayUrl:directAddrs:to:maxSize:progress:)` | Download a blob by hash from one node into a file (verified, size-limited, cancellable) |
| `pushToNode(nodeId:relayUrl:directAddrs:data:)` | Push bytes to a node, return the blob hash (`directAddrs`: `ip:port` or `host:port`) |
| `pushToNode(nodeId:relayUrl:directAddrs:fileURL:)` | Push a file to a node without loading it into memory, return the blob hash |
| `endpointID()` | This node's Ed25519 public key, 64 lowercase hex (same as `info().nodeId`) |
| `sign(_:)` | 64-byte Ed25519 signature with the endpoint secret key |
| `tagBlob(hash:name:format:)` | Pin a blob to prevent GC |
| `untagBlob(name:)` | Remove a pin |
| `createTicket(hash:format:)` | Create a ticket for an existing blob |
| `info()` | Get node ID, relay URL, direct addresses, connection status |
| `close()` | Gracefully shut down the node |

### IrohConfig

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `storagePath` | `URL` | Application Support/iroh | Blob storage directory |
| `relayEnabled` | `Bool` | `true` | Use n0 public relay servers |
| `customRelayUrl` | `String?` | `nil` | Custom relay server URL |
| `servesBlobs` | `Bool` | `true` | Serve blobs to peers; `false` makes a client-only node (fetch and push still work) |
| `acceptsPushes` | `Bool` | `false` | Let peers push blobs into this node's store; requires `servesBlobs` |
| `dhtLookup` | `Bool` | `false` | Resolve endpoint IDs on the Mainline DHT, so `fetch` / `pushToNode` work with an ID alone |
| `dhtPublish` | `Bool` | `false` | Also publish this node's address to the DHT; requires `dhtLookup` and `servesBlobs` |

### IrohBLAKE3

| Method | Description |
|--------|-------------|
| `hex(_:)` | BLAKE3 hash of `Data` as 64 lowercase hex (equals the raw blob hash) |

## CLI Demo

```bash
# Blob storage demo
swift run iroh-cli blob
```

## Building from Source

### Prerequisites

- Rust toolchain (`rustup`)
- Xcode 26+

### Build XCFramework

```bash
./scripts/build-xcframework.sh
```

Targets:
- `aarch64-apple-ios` (iOS device, arm64)
- `aarch64-apple-ios-sim` + `x86_64-apple-ios` (iOS Simulator, universal arm64 + x86_64)
- `aarch64-apple-darwin` + `x86_64-apple-darwin` (macOS, universal arm64 + x86_64)

### Run Tests

```bash
IROH_LOCAL_DEV=1 swift test
```

## Architecture

```
Swift Layer (IrohSwift)           Rust FFI Layer              Iroh Libraries
┌─────────────────────┐          ┌──────────────────┐        ┌────────────────────┐
│ IrohNode (actor)    │──C ABI──▶│ ffi.rs           │───────▶│ iroh 1.4.1         │
│ IrohConfig          │          │ node.rs          │        │ iroh-blobs 0.104.1 │
└─────────────────────┘          └──────────────────┘        └────────────────────┘
```

- **Swift Actor**: Thread-safe with async/await
- **Rust FFI**: Callback-based async bridged via `CheckedContinuation`
- **Storage**: Persistent `FsStore` (excluded from iCloud backup)

## Breaking Changes in 0.8.0

Docs support is removed. iroh-docs had no release on iroh-blobs 0.104, and no
known app enabled it. Removed:

- `IrohDoc`, `IrohAuthor`, `DocEntry`, `DocEvent`, `DocShareMode` and
  `KeychainAccessibility`.
- `IrohConfig.docsEnabled` (drop the `docsEnabled:` argument), and
  `IrohNode.createDoc()`, `joinDoc(ticket:)` and `importAuthor(_:)`.
- The `IrohError` cases `docsNotEnabled`, `docCreationFailed`, `docJoinFailed`,
  `docClosed`, `docGetFailed`, `docSetFailed`, `docDeleteFailed`,
  `docShareFailed`, `contentReadFailed`, `docSubscribeFailed`,
  `authorCreationFailed`, `authorImportFailed` and `keychainError`.
- C API: `IrohNodeConfig.docs_enabled` and every `iroh_author_*` and `iroh_doc_*`
  export.

A `docs` folder that a docs-enabled node left in its storage directory is no
longer read, and authors saved in the Keychain stay there; delete them if you
no longer need them.

iroh-blobs moves to 0.104.1. The wire protocol is unchanged (ALPN
`/iroh-bytes/4`), so 0.8.0 and 0.7.x nodes still exchange blobs.

## Breaking Changes in 0.7.0

- `KeychainAccessibility.always` is removed (Apple deprecated
  `kSecAttrAccessibleAlways`). Use `.afterFirstUnlock`. Authors saved with
  `.always` still load.
- `IrohDoc.get(key:)` and `getMany(prefix:)` return the latest entry per key
  across all authors, and hide a key whose latest entry is a deletion. Before,
  results were sorted by author: `get` could return an older author's value
  (even after another author deleted the key), and `getMany` returned one entry
  per author per key and kept only the newest 100 matches.
- `delete(author:key:)` is documented as what it always was: a prefix delete.
- `putWithRetry` / `getWithRetry` throw `CancellationError` and `nodeClosed`
  as they are, without retrying or wrapping them in `maxRetriesExceeded`.
- `IrohAuthor.getOrCreate` throws `keychainError` when the stored author cannot
  be read (e.g. the device is locked) instead of replacing it with a new one.

## Behavior Change in 0.6.0

Serving nodes no longer accept pushes unless `IrohConfig.acceptsPushes` is set
(default `false`). Up to 0.5.x any peer could push blobs into a serving node's
store. If your app receives pushes from other iroh-swift nodes, create the
receiving node with `acceptsPushes: true`. Pushing *to* other nodes, fetching and
serving blobs are unchanged.

Also new in 0.6.0: `directAddrs` accepts `host:port` (resolved over
DNS-over-HTTPS first, then the system resolver), and `IrohConfig.dhtLookup` /
`dhtPublish` opt in to endpoint-ID lookup on the Mainline DHT. Both are additive;
existing `ip:port` addresses behave as before.

## Version Compatibility

| iroh-swift | iroh | iroh-blobs | iroh-docs |
|------------|------|------------|-----------|
| 0.8.x | 1.4.1 (+ iroh-mainline-address-lookup 0.6) | 0.104.1 | - |
| 0.7.x | 1.4.1 (+ iroh-mainline-address-lookup 0.6) | 0.103.1 | 0.101 |
| 0.6.x | 1.3.0 (+ iroh-mainline-address-lookup 0.6) | 0.103.1 | 0.101 |
| 0.5.x | 1.1.0 | 0.103 | 0.101 |
| 0.4.x | 0.97 | 0.99 | 0.97 |
| 0.3.x | 0.95 | 0.97 | 0.32 |
| 0.2.x | 0.95 | 0.97 | - |
| 0.1.x | 0.95 | 0.97 | - |

## License

Apache-2.0 OR MIT
