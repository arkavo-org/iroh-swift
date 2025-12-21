import Foundation

/// Progress information during a download operation.
public struct DownloadProgress: Sendable {
    /// Bytes downloaded so far.
    public let downloaded: UInt64
    /// Total bytes expected (0 if unknown).
    public let total: UInt64

    /// Progress as a fraction (0.0 to 1.0), or nil if total is unknown.
    public var fraction: Double? {
        guard total > 0 else { return nil }
        return Double(downloaded) / Double(total)
    }
}

/// Information about an Iroh node.
public struct NodeInfo: Sendable {
    /// The node's unique identifier.
    public let nodeId: String
    /// The relay server URL, if connected.
    public let relayUrl: String?
    /// Whether the node is connected to the network.
    public let isConnected: Bool
}

/// Parsed ticket information.
public struct TicketInfo: Sendable {
    /// Whether the ticket string is valid.
    public let isValid: Bool
    /// The blob hash, if valid.
    public let hash: String?
    /// The source node ID, if valid.
    public let nodeId: String?
    /// Whether this is a recursive (collection) ticket.
    public let isRecursive: Bool
}
