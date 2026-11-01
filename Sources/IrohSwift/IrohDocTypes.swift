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
    ///
    /// Returns `nil` if the content hash is null, or the key is null with a
    /// non-zero length.
    init?(from ffiEntry: IrohDocEntry) {
        guard let hashPtr = ffiEntry.content_hash else { return nil }
        let keyLen = Int(ffiEntry.key.len)
        if let keyPtr = ffiEntry.key.data {
            self.key = Data(bytes: keyPtr, count: keyLen)
        } else if keyLen == 0 {
            self.key = Data()
        } else {
            return nil
        }

        // Convert author ID bytes to hex string
        let authorBytes = withUnsafeBytes(of: ffiEntry.author_id.bytes) { Data($0) }
        self.authorId = authorBytes.map { String(format: "%02x", $0) }.joined()

        self.contentHash = String(cString: hashPtr)

        self.contentSize = ffiEntry.content_size
        self.timestamp = ffiEntry.timestamp
    }

    /// Convert an FFI entry and free it with `iroh_doc_entry_free`.
    ///
    /// Returns `nil` for a null pointer or a malformed entry (which is still freed).
    static func consume(_ entryPtr: UnsafePointer<IrohDocEntry>?) -> DocEntry? {
        guard let entryPtr else { return nil }
        let entry = DocEntry(from: entryPtr.pointee)
        iroh_doc_entry_free(UnsafeMutablePointer(mutating: entryPtr))
        return entry
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
    ///
    /// Returns `nil` for an event type this version does not know, or when a
    /// field the type requires is null.
    static func from(_ ffiEvent: IrohDocEvent) -> DocEvent? {
        // Decoded only by the insert cases, which carry an entry
        var entry: DocEntry? { ffiEvent.entry.flatMap { DocEntry(from: $0.pointee) } }
        let peerId = ffiEvent.peer_id.map { String(cString: $0) }

        switch ffiEvent.event_type {
        case InsertLocal:
            return entry.map { .insertLocal($0) }

        case InsertRemote:
            guard let entry, let peerId else { return nil }
            return .insertRemote(from: peerId, entry: entry)

        case ContentReady:
            return ffiEvent.content_hash.map { .contentReady(hash: String(cString: $0)) }

        case PendingContentReady:
            return .pendingContentReady

        case NeighborUp:
            return peerId.map { .neighborUp(peerId: $0) }

        case NeighborDown:
            return peerId.map { .neighborDown(peerId: $0) }

        case SyncFinished:
            return peerId.map { .syncFinished(peerId: $0) }

        default:
            return nil
        }
    }
}
