# Network Discovery and Relay Servers

This document explains how Iroh handles network connectivity, NAT traversal, and relay servers.

## Overview

Iroh uses QUIC (UDP-based) for peer-to-peer communication. To enable connectivity between devices behind NATs and firewalls, Iroh employs a combination of:

1. **Direct connections** when possible
2. **Relay servers** for NAT traversal and as a fallback

## How Connection Establishment Works

When two nodes want to communicate:

1. **Address Discovery**: Each node publishes its addresses (IP, port, relay URLs) to allow peers to find them
2. **Hole Punching**: Iroh attempts UDP hole punching to establish a direct connection
3. **Relay Fallback**: If direct connection fails, traffic is routed through a relay server

## Relay Servers

### What Relay Servers Do

- **STUN-like NAT Traversal**: Help nodes discover their public IP and port
- **Traffic Relay**: Route encrypted traffic when direct connections aren't possible
- **Rendezvous Point**: Allow nodes to find each other even with dynamic IPs

### Default Relays (n0 Public Network)

By default, IrohSwift uses n0's public relay servers. These are:
- Free for development and small-scale use
- Globally distributed for low latency
- Automatically selected based on geography

### Custom Relay Servers

For production or private deployments, you can specify a custom relay:

```swift
let config = IrohConfig(
    customRelayUrl: "https://relay.your-domain.com"
)
let node = try await IrohNode(config: config)
```

**When to use custom relays:**
- Enterprise deployments requiring data sovereignty
- High-volume applications needing dedicated infrastructure
- Private networks without public internet access

## Direct vs Relay Connections

### Direct Connections Work When:
- Both devices are on the same local network (LAN)
- Neither device is behind a restrictive NAT (symmetric NAT)
- Firewall allows UDP traffic on the required ports

### Relay is Used When:
- Devices are behind symmetric NATs
- Corporate firewalls block UDP hole punching
- One device has no public connectivity

### Checking Connection Status

```swift
let info = try await node.info()
print("Node ID: \(info.nodeId)")
print("Connected to relay: \(info.relayUrl ?? "None")")
print("Is connected: \(info.isConnected)")
```

## Network Troubleshooting

### Connection Timeouts

If `get()` operations time out:
1. Check that the source node is still online
2. Verify the ticket is valid and not stale
3. Use the timeout option to avoid indefinite hangs:

```swift
let data = try await node.get(
    ticket: ticket,
    options: OperationOptions(timeout: .seconds(30))
)
```

### Firewall Considerations

Iroh works best when:
- UDP traffic is allowed (QUIC uses UDP)
- Outbound connections to relay servers are permitted
- The relay port (typically 443) is open

### Disabling Relay

For testing or specific network configurations, you can disable relay:

```swift
let config = IrohConfig(relayEnabled: false)
let node = try await IrohNode(config: config)
```

**Note:** Without relay, connections will only work on local networks or when both devices have public IPs.

## Performance Implications

| Connection Type | Latency | Bandwidth | Use Case |
|-----------------|---------|-----------|----------|
| Direct (LAN) | ~1ms | Full | Same network |
| Direct (WAN) | Variable | Full | Public IPs |
| Relay | +10-50ms | Limited | NAT traversal |

## Security

- All traffic is encrypted end-to-end regardless of connection type
- Relay servers cannot read the content being transferred
- Node identities are cryptographic (Ed25519 key pairs)
