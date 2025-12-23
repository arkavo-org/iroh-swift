import Foundation
import IrohSwiftFFI

/// Share mode for document tickets.
public enum DocShareMode: UInt8, Sendable {
    /// Read-only access to the document.
    case read = 0
    /// Read and write access to the document.
    case write = 1

    /// Convert to FFI type.
    var ffiMode: IrohDocShareMode {
        switch self {
        case .read: return Read
        case .write: return Write
        }
    }
}

/// A document entry (key-value pair with metadata).
///
/// Entries contain metadata about a key-value pair, including who wrote it,
/// when it was written, and a hash of the content. To get the actual content,
/// use the `content(from:)` method.
public struct DocEntry: Sendable {
    /// The author ID who wrote this entry (64-character hex string).
    public let authorId: String

    /// The key bytes.
    public let key: Data

    /// The content hash as a hex string.
    public let contentHash: String

    /// Size of the content in bytes.
    public let contentSize: UInt64

    /// Timestamp when entry was created (microseconds since epoch).
    public let timestamp: UInt64

    /// The key as a UTF-8 string, if valid.
    public var keyString: String? {
        String(data: key, encoding: .utf8)
    }

    /// Fetch the actual content data for this entry.
    ///
    /// Entries only contain a hash of the content. Use this method to
    /// retrieve the actual bytes.
    ///
    /// - Parameter doc: The document to read content from.
    /// - Returns: The content data.
    /// - Throws: `IrohError.contentReadFailed` if reading fails.
    public func content(from doc: IrohDoc) async throws -> Data {
        try await doc.readContent(hash: contentHash)
    }

    /// Create from FFI entry.
    init(from ffiEntry: IrohDocEntry) {
        // Convert author ID bytes to hex string
        let authorBytes = withUnsafeBytes(of: ffiEntry.author_id.bytes) { Data($0) }
        self.authorId = authorBytes.map { String(format: "%02x", $0) }.joined()

        // Copy key bytes
        self.key = Data(bytes: ffiEntry.key.data, count: Int(ffiEntry.key.len))

        // Copy content hash string
        self.contentHash = String(cString: ffiEntry.content_hash)

        self.contentSize = ffiEntry.content_size
        self.timestamp = ffiEntry.timestamp
    }
}

/// Events from document subscriptions.
public enum DocEvent: Sendable {
    /// A local entry was inserted.
    case insertLocal(DocEntry)

    /// A remote entry was received from a peer.
    case insertRemote(from: String, entry: DocEntry)

    /// Content is now available locally (finished downloading).
    case contentReady(hash: String)

    /// All pending content is now ready.
    case pendingContentReady

    /// A new peer joined the document swarm.
    case neighborUp(peerId: String)

    /// A peer left the document swarm.
    case neighborDown(peerId: String)

    /// Sync finished with a peer.
    case syncFinished(peerId: String)

    /// Create from FFI event.
    static func from(_ ffiEvent: IrohDocEvent) -> DocEvent {
        switch ffiEvent.event_type {
        case InsertLocal:
            let entry = DocEntry(from: ffiEvent.entry!.pointee)
            return .insertLocal(entry)

        case InsertRemote:
            let entry = DocEntry(from: ffiEvent.entry!.pointee)
            let peerId = String(cString: ffiEvent.peer_id!)
            return .insertRemote(from: peerId, entry: entry)

        case ContentReady:
            let hash = String(cString: ffiEvent.content_hash!)
            return .contentReady(hash: hash)

        case PendingContentReady:
            return .pendingContentReady

        case NeighborUp:
            let peerId = String(cString: ffiEvent.peer_id!)
            return .neighborUp(peerId: peerId)

        case NeighborDown:
            let peerId = String(cString: ffiEvent.peer_id!)
            return .neighborDown(peerId: peerId)

        case SyncFinished:
            let peerId = String(cString: ffiEvent.peer_id!)
            return .syncFinished(peerId: peerId)

        default:
            fatalError("Unknown document event type: \(ffiEvent.event_type)")
        }
    }
}
