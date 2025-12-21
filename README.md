# iroh-swift

Minimal Swift bindings for [Iroh](https://iroh.computer) blob storage.

## Requirements

- Swift 6.2+
- iOS 26+ / macOS 26+
- Xcode 26+

## Installation

### Swift Package Manager

Add to your `Package.swift`:

```swift
dependencies: [
    .package(url: "https://github.com/arkavo-org/iroh-swift.git", from: "0.1.0")
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

### Basic Example

```swift
import IrohSwift

// Create a node (uses Application Support for storage by default)
let node = try await IrohNode()

// Store data and get a shareable ticket
let data = "Hello, Iroh!".data(using: .utf8)!
let ticket = try await node.put(data)
print("Ticket: \(ticket)")

// Retrieve data using a ticket
let retrieved = try await node.get(ticket: ticket)
print("Data: \(String(data: retrieved, encoding: .utf8)!)")
```

### Custom Configuration

```swift
import IrohSwift

// Custom storage path and relay settings
let config = IrohConfig(
    storagePath: myCustomURL,  // nil = Application Support/iroh
    relayEnabled: true         // n0 public relays (default: true)
)

let node = try await IrohNode(config: config)
```

## API

### IrohNode

An actor providing thread-safe access to Iroh blob operations.

| Method | Description |
|--------|-------------|
| `init(config:)` | Create a new node with optional configuration |
| `put(_:)` | Store data and return a shareable ticket |
| `get(ticket:)` | Download data using a ticket |

### IrohConfig

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `storagePath` | `URL` | Application Support/iroh | Blob storage directory |
| `relayEnabled` | `Bool` | `true` | Use n0 public relay servers |

### IrohError

| Case | Description |
|------|-------------|
| `nodeCreationFailed` | Failed to create the Iroh node |
| `putFailed` | Failed to store data |
| `getFailed` | Failed to retrieve data |
| `invalidTicket` | Invalid ticket format |

## Building from Source

### Prerequisites

- Rust toolchain (`rustup`)
- Xcode 26+

### Build XCFramework

```bash
./scripts/build-xcframework.sh
```

This builds for:
- `aarch64-apple-ios` (iOS device)
- `aarch64-apple-ios-sim` (iOS Simulator)
- `aarch64-apple-darwin` (macOS)

### Run Tests

```bash
swift test
```

## Architecture

- **Rust FFI**: Manual C ABI using `cbindgen` for header generation
- **Swift Actor**: Thread-safe wrapper with async/await
- **Storage**: Persistent `FsStore` in Application Support (excluded from iCloud backup)
- **Networking**: n0 public relays enabled by default for NAT traversal

## Version Compatibility

| iroh-swift | iroh | iroh-blobs |
|------------|------|------------|
| 0.1.x | 0.95 | 0.97 |

## License

Apache-2.0 OR MIT