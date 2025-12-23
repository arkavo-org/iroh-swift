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
///
/// A ticket is a self-contained string that encodes everything needed to download a blob:
///
/// **Components:**
/// - **Hash**: Blake3 content hash (32 bytes) - uniquely identifies the content
/// - **Node ID**: Ed25519 public key of the source node
/// - **Address hints**: Relay URLs and IP addresses for connectivity
/// - **Format flag**: Whether it's a single blob or recursive collection
///
/// **Example ticket format:**
/// ```
/// blobafk2bi...  (typically 100+ characters)
/// ```
///
/// **Properties:**
/// - Tickets are **content-addressed**: the hash identifies the exact content
/// - Tickets don't expire, but the source node must be online to download
/// - Tickets are safe to share publicly (content is verified on download)
///
/// See `docs/TICKETS.md` for detailed documentation.
public struct TicketInfo: Sendable {
    /// Whether the ticket string is valid and could be parsed.
    public let isValid: Bool

    /// The blake3 content hash as a hex string.
    /// This uniquely identifies the blob content.
    /// Nil if the ticket is invalid.
    public let hash: String?

    /// The source node's unique identifier (Ed25519 public key as hex).
    /// This identifies which node originally created the ticket.
    /// Nil if the ticket is invalid.
    public let nodeId: String?

    /// Whether this ticket points to a recursive collection.
    /// A recursive ticket can contain multiple related blobs.
    public let isRecursive: Bool
}

/// Format of blob data.
///
/// Determines how the blob content is interpreted:
/// - `.raw`: A single blob (most common case)
/// - `.hashSeq`: A collection of blobs referenced by their hashes
public enum BlobFormat: UInt8, Sendable {
    /// Raw single blob.
    case raw = 0
    /// Hash sequence (collection of blobs).
    case hashSeq = 1
}
