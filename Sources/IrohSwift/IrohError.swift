import Foundation

/// Errors that can occur when using IrohNode.
public enum IrohError: Error, Sendable {
    /// Failed to create the Iroh node.
    case nodeCreationFailed(String)
    /// Failed to add bytes to the blob store.
    case putFailed(String)
    /// Failed to download bytes from a ticket.
    case getFailed(String)
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
        }
    }
}
