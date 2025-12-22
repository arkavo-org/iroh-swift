# Ticket Format and Usage

This document explains the structure and usage of Iroh blob tickets.

## What is a Ticket?

A ticket is a self-contained string that includes everything needed to download a blob:

- **Content hash**: Identifies the exact content (blake3, 32 bytes)
- **Node address**: How to reach the source node
- **Format**: Whether it's a single blob or a collection

## Ticket Structure

Tickets are encoded as URL-safe strings starting with `blob`:

```
blobafk2bi...  (typically 100+ characters)
```

### Components Encoded

1. **Hash** (32 bytes): Blake3 hash of the content
2. **Node ID** (32 bytes): Ed25519 public key of the source node
3. **Address hints**: Relay URLs, IP addresses
4. **Format flag**: Single blob vs recursive collection

### Example Breakdown

```
blob          - Protocol prefix
a             - Format indicator
fk2bi...      - Base32 encoded data containing:
                - Content hash
                - Node addressing info
                - Network hints
```

## Ticket Lifecycle

### Creation

When you call `put()`, Iroh:
1. Stores the data locally
2. Computes the blake3 hash
3. Generates a ticket with your node's current address

```swift
let data = "Hello, World!".data(using: .utf8)!
let ticket = try await node.put(data)
// ticket: "blobafk2bi..."
```

### Sharing

Tickets can be shared via any channel:
- QR codes
- URLs
- Messaging apps
- Local network broadcast

### Redemption

When you call `get(ticket:)`, Iroh:
1. Parses the ticket
2. Connects to the source node
3. Downloads and verifies the content
4. Returns the data

```swift
let data = try await node.get(ticket: ticket)
```

## Validation

You can validate a ticket without downloading:

```swift
let info = await validateTicket(ticket)

if info.isValid {
    print("Hash: \(info.hash!)")
    print("Node ID: \(info.nodeId!)")
    print("Recursive: \(info.isRecursive)")
} else {
    print("Invalid ticket format")
}
```

## Ticket Properties

### Content-Addressed

Tickets are **content-addressed**, meaning:
- The hash uniquely identifies the content
- Any node with the same content can serve it
- Content cannot be modified without changing the hash

### No Expiration

Tickets themselves don't expire, but:
- The source node must be online to download
- Network address hints may become stale
- Content may be deleted from the source

### Security

- Tickets don't contain secrets
- They're safe to share publicly
- Content is verified on download (hash check)
- Transport is encrypted (QUIC/TLS)

## Advanced Usage

### Recursive Tickets

For collections of files:

```swift
if info.isRecursive {
    // This ticket points to a collection
    // Individual files can be accessed within
}
```

### Ticket Info in UI

```swift
struct TicketPreview: View {
    let ticket: String
    @State private var info: TicketInfo?

    var body: some View {
        Group {
            if let info = info, info.isValid {
                VStack {
                    Text("Hash: \(info.hash!.prefix(16))...")
                    Text("From: \(info.nodeId!.prefix(16))...")
                }
            } else {
                Text("Invalid ticket")
            }
        }
        .task {
            info = await validateTicket(ticket)
        }
    }
}
```

## Common Issues

### "Failed to parse ticket"

The ticket string is malformed:
- Check for truncation when copying
- Ensure no extra whitespace
- Verify the `blob` prefix is present

### "Failed to download blob"

- Source node may be offline
- Network connectivity issues
- Use timeout to avoid indefinite hangs:

```swift
let data = try await node.get(
    ticket: ticket,
    options: OperationOptions(timeout: .seconds(30))
)
```

### Stale Address Hints

If a node's IP changes frequently:
- Relay connection helps maintain reachability
- Fresh tickets have updated address hints
- Consider implementing ticket refresh for long-lived content
