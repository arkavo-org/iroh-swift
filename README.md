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
    .package(url: "https://github.com/arkavo-org/iroh-swift.git", from: "0.3.0")
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
| `createDoc()` | Create a new document (requires `docsEnabled`) |
| `joinDoc(ticket:)` | Join an existing document |
| `importAuthor(_:)` | Register an author with the docs engine |
| `tagBlob(hash:name:format:)` | Pin a blob to prevent GC |
| `untagBlob(name:)` | Remove a pin |
| `createTicket(hash:format:)` | Create a ticket for an existing blob |
| `info()` | Get node ID, relay URL, connection status |
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
- `aarch64-apple-ios` (iOS device)
- `aarch64-apple-ios-sim` (iOS Simulator)
- `aarch64-apple-darwin` (macOS)

### Run Tests

```bash
swift test
```

## Architecture

```
Swift Layer (IrohSwift)           Rust FFI Layer              Iroh Libraries
┌─────────────────────┐          ┌──────────────────┐        ┌─────────────┐
│ IrohNode (actor)    │──C ABI──▶│ ffi.rs           │──────▶│ iroh 0.95   │
│ IrohDoc (actor)     │          │ node.rs          │       │ iroh-blobs  │
│ IrohAuthor          │          └──────────────────┘       │ iroh-docs   │
└─────────────────────┘                                      └─────────────┘
```

- **Swift Actor**: Thread-safe with async/await
- **Rust FFI**: Callback-based async bridged via `CheckedContinuation`
- **Storage**: Persistent `FsStore` (excluded from iCloud backup)

## Version Compatibility

| iroh-swift | iroh | iroh-blobs | iroh-docs |
|------------|------|------------|-----------|
| 0.3.x | 0.95 | 0.97 | 0.32 |
| 0.2.x | 0.95 | 0.97 | - |
| 0.1.x | 0.95 | 0.97 | - |

## License

Apache-2.0 OR MIT
