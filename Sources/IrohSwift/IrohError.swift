import Foundation

/// Errors that can occur when using IrohNode.
public enum IrohError: Error, Sendable {
    /// Failed to create the Iroh node.
    case nodeCreationFailed(String)
    /// Failed to add bytes to the blob store.
    case putFailed(String)
    /// Failed to download bytes from a ticket.
    case getFailed(String)
    /// Failed to push data to a remote node.
    case pushFailed(String)
    /// Invalid ticket format.
    case invalidTicket(String)
    /// Failed to encode string with the specified encoding.
    case stringEncodingFailed(String.Encoding)
    /// Failed to decode data as string with the specified encoding.
    case stringDecodingFailed(String.Encoding)
    /// Failed to encode value to JSON.
    case encodingFailed(String)
    /// Failed to decode JSON to value.
    case decodingFailed(String)
    /// Operation failed after maximum retry attempts.
    case maxRetriesExceeded(attempts: Int, lastError: any Error)
    /// Invalid configuration.
    case invalidConfiguration(String)
    /// Operation timed out.
    case timeout
    /// Node has been closed.
    case nodeClosed
    /// Failed to close the node.
    case closeFailed(String)
    // MARK: - Blob Errors
    /// Failed to tag (pin) a blob.
    case blobTagFailed(String)
    /// Failed to remove a tag (unpin) from a blob.
    case blobUntagFailed(String)
    /// Failed to create a ticket for a blob.
    case ticketCreationFailed(String)
    // MARK: - Fetch Errors
    /// The remote node does not have the requested blob.
    case notFound
    /// The remote node announced a blob of this many bytes, more than the
    /// caller's `maxSize`. Nothing was written.
    case sizeLimitExceeded(UInt64)
    /// Failed to fetch a blob from a node (bad arguments, connection,
    /// protocol, verification or file I/O).
    case fetchFailed(String)
}

extension IrohError: LocalizedError {
    public var errorDescription: String? {
        switch self {
        case .nodeCreationFailed(let msg):
            return "Failed to create Iroh node: \(msg)"
        case .putFailed(let msg):
            return "Failed to put bytes: \(msg)"
        case .getFailed(let msg):
            return "Failed to get bytes: \(msg)"
        case .pushFailed(let msg):
            return "Failed to push data: \(msg)"
        case .invalidTicket(let msg):
            return "Invalid ticket: \(msg)"
        case .stringEncodingFailed(let encoding):
            return "Failed to encode string using \(encoding)"
        case .stringDecodingFailed(let encoding):
            return "Failed to decode data as string using \(encoding)"
        case .encodingFailed(let msg):
            return "Failed to encode value: \(msg)"
        case .decodingFailed(let msg):
            return "Failed to decode value: \(msg)"
        case .maxRetriesExceeded(let attempts, let lastError):
            return "Operation failed after \(attempts) attempts: \(lastError.localizedDescription)"
        case .invalidConfiguration(let msg):
            return "Invalid configuration: \(msg)"
        case .timeout:
            return "Operation timed out"
        case .nodeClosed:
            return "Node has been closed"
        case .closeFailed(let msg):
            return "Failed to close node: \(msg)"
        case .blobTagFailed(let msg):
            return "Failed to tag blob: \(msg)"
        case .blobUntagFailed(let msg):
            return "Failed to untag blob: \(msg)"
        case .ticketCreationFailed(let msg):
            return "Failed to create ticket: \(msg)"
        case .notFound:
            return "Blob not found on the remote node"
        case .sizeLimitExceeded(let size):
            return "Blob size \(size) bytes exceeds the size limit"
        case .fetchFailed(let msg):
            return "Failed to fetch blob: \(msg)"
        }
    }
}
