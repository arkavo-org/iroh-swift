# iroh-swift

Swift bindings for [Iroh](https://iroh.computer) - peer-to-peer blob storage and document sync.

## Features

- **Blob Storage**: Store and retrieve content-addressed data
- **Docs**: Sync key-value documents across peers
- **Relay Support**: NAT traversal via n0 public relays
- **Keychain Integration**: Secure author key storage on iOS/macOS

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
them returns an address, the device's system resolver (up to 5 s; it also honours
the hosts file, so `localhost` and VPN/enterprise names work). Each node builds
the resolvers once and reuses them. Hostnames must be ASCII (use punycode for
internationalized names).

A hostname cannot impersonate a node: you pin the endpoint ID and the QUIC
handshake checks it, so a wrong or poisoned DNS answer only makes the connection
fail. That makes a CNAME that follows a redeploy, such as `iroh.arkavo.net`, safe
to use.

Failure: a malformed entry (`host-without-port`, `host:0`, an unbracketed IPv6
address) fails the call before anything is sent. If hostnames were given, none
resolved, and there is no `relayUrl` and no literal address left, the call fails
with a message naming the hosts: `IrohError.fetchFailed` for `fetch`,
`IrohError.pushFailed` for `pushToNode`. A hostname that fails is ignored when a
relay URL or another address is available. Cancelling the task also cancels a
slow lookup.

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

### Docs (Synchronized Documents)

```swift
import IrohSwift

// Create a node with docs enabled
let config = IrohConfig(
    storagePath: myStorageURL,
    relayEnabled: true,
    docsEnabled: true
)
let node = try await IrohNode(config: config)

// Create or load an author (keys stored in Keychain)
let author = try await IrohAuthor.getOrCreate(identifier: "my-app")
try await node.importAuthor(author)

// Create a document
let doc = try await node.createDoc()

// Write key-value pairs
try await doc.set(author: author, key: "greeting", value: "Hello!".data(using: .utf8)!)

// Read values
if let entry = try await doc.get(key: "greeting") {
    let content = try await entry.content(from: doc)
    print(String(data: content, encoding: .utf8)!)
}

// Share the document
let ticket = try await doc.shareTicket(mode: .write)
```

### Join a Document

```swift
// Join using a ticket from another peer
let doc = try await node.joinDoc(ticket: shareTicket)

// Subscribe to live updates
for try await event in try await doc.subscribe() {
    switch event {
    case .insertRemote(let peer, let entry):
        print("Received from \(peer): \(entry.key)")
    case .syncFinished(let peer):
        print("Sync complete with \(peer)")
    default:
        break
    }
}
```

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

### Author Management

```swift
// Create a new author (not stored)
let author = try await IrohAuthor.create()

// Get or create with Keychain storage
let author = try await IrohAuthor.getOrCreate(
    identifier: "default",
    accessibility: .afterFirstUnlock  // Keychain security level
)

// Import from backup
let author = try await IrohAuthor.fromHex(secretHex, saveTo: "restored")

// Export for backup (handle securely!)
let secretHex = author.exportSecretHex()

// Check/delete from Keychain
if IrohAuthor.exists(identifier: "default") {
    try IrohAuthor.delete(identifier: "default")
}
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
| `createDoc()` | Create a new document (requires `docsEnabled`) |
| `joinDoc(ticket:)` | Join an existing document |
| `importAuthor(_:)` | Register an author with the docs engine |
| `tagBlob(hash:name:format:)` | Pin a blob to prevent GC |
| `untagBlob(name:)` | Remove a pin |
| `createTicket(hash:format:)` | Create a ticket for an existing blob |
| `info()` | Get node ID, relay URL, direct addresses, connection status |
| `close()` | Gracefully shut down the node |

### IrohDoc

| Method | Description |
|--------|-------------|
| `set(author:key:value:)` | Write a key-value pair |
| `get(key:)` | Read a single entry |
| `getMany(prefix:)` | Query entries by key prefix |
| `delete(author:key:)` | Delete an entry |
| `shareTicket(mode:)` | Get a shareable ticket (.read or .write) |
| `subscribe()` | Subscribe to live document events |

### IrohAuthor

| Method | Description |
|--------|-------------|
| `create()` | Create a new random author |
| `getOrCreate(identifier:accessibility:)` | Load from or save to Keychain |
| `fromHex(_:saveTo:accessibility:)` | Import from hex-encoded secret |
| `exportSecretHex()` | Export secret key (handle securely!) |
| `exists(identifier:)` | Check if author exists in Keychain |
| `delete(identifier:)` | Remove author from Keychain |

### IrohConfig

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `storagePath` | `URL` | Application Support/iroh | Blob storage directory |
| `relayEnabled` | `Bool` | `true` | Use n0 public relay servers |
| `docsEnabled` | `Bool` | `false` | Enable document sync |
| `customRelayUrl` | `String?` | `nil` | Custom relay server URL |
| `servesBlobs` | `Bool` | `true` | Serve blobs to peers; `false` makes a client-only node (fetch and push still work) |
| `acceptsPushes` | `Bool` | `false` | Let peers push blobs into this node's store; requires `servesBlobs` |
| `dhtLookup` | `Bool` | `false` | Resolve endpoint IDs on the Mainline DHT, so `fetch` / `pushToNode` work with an ID alone |
| `dhtPublish` | `Bool` | `false` | Also publish this node's address to the DHT; requires `dhtLookup` and `servesBlobs` |

### IrohBLAKE3

| Method | Description |
|--------|-------------|
| `hex(_:)` | BLAKE3 hash of `Data` as 64 lowercase hex (equals the raw blob hash) |

### KeychainAccessibility

| Case | Description |
|------|-------------|
| `.afterFirstUnlock` | Available after first unlock (default, recommended) |
| `.whenUnlocked` | Only when device is unlocked |
| `.always` | Always available (less secure) |

## CLI Demo

```bash
# Blob storage demo
swift run iroh-cli blob

# Create a document and share
swift run iroh-cli docs

# Join a document
swift run iroh-cli docs-join <ticket>
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
│ IrohNode (actor)    │──C ABI──▶│ ffi.rs           │───────▶│ iroh 1.3.0         │
│ IrohDoc (actor)     │          │ node.rs          │        │ iroh-blobs 0.103.1 │
│ IrohAuthor          │          └──────────────────┘        │ iroh-docs 0.101    │
└─────────────────────┘                                      └────────────────────┘
```

- **Swift Actor**: Thread-safe with async/await
- **Rust FFI**: Callback-based async bridged via `CheckedContinuation`
- **Storage**: Persistent `FsStore` (excluded from iCloud backup)

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
| 0.6.x | 1.3.0 (+ iroh-mainline-address-lookup 0.6) | 0.103.1 | 0.101 |
| 0.5.x | 1.1.0 | 0.103 | 0.101 |
| 0.4.x | 0.97 | 0.99 | 0.97 |
| 0.3.x | 0.95 | 0.97 | 0.32 |
| 0.2.x | 0.95 | 0.97 | - |
| 0.1.x | 0.95 | 0.97 | - |

## License

Apache-2.0 OR MIT
