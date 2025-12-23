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
    /// Invalid configuration.
    case invalidConfiguration(String)
    /// Operation timed out.
    case timeout
    /// Node has been closed.
    case nodeClosed
    /// Failed to close the node.
    case closeFailed(String)
    // MARK: - Docs Errors
    /// Docs is not enabled on this node.
    case docsNotEnabled
    /// Failed to create a document.
    case docCreationFailed(String)
    /// Failed to join a document.
    case docJoinFailed(String)
    /// Document has been closed.
    case docClosed
    /// Failed to get entry from document.
    case docGetFailed(String)
    /// Failed to set entry in document.
    case docSetFailed(String)
    /// Failed to delete entry from document.
    case docDeleteFailed(String)
    /// Failed to share document.
    case docShareFailed(String)
    /// Failed to read content from store.
    case contentReadFailed(String)
    /// Failed to subscribe to document events.
    case docSubscribeFailed(String)
    // MARK: - Author Errors
    /// Failed to create author.
    case authorCreationFailed(String)
    /// Failed to import author into docs engine.
    case authorImportFailed(String)
    /// Keychain operation failed.
    case keychainError(String)
    // MARK: - Blob Errors
    /// Failed to tag (pin) a blob.
    case blobTagFailed(String)
    /// Failed to create a ticket for a blob.
    case ticketCreationFailed(String)
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
        case .invalidConfiguration(let msg):
            return "Invalid configuration: \(msg)"
        case .timeout:
            return "Operation timed out"
        case .nodeClosed:
            return "Node has been closed"
        case .closeFailed(let msg):
            return "Failed to close node: \(msg)"
        case .docsNotEnabled:
            return "Docs is not enabled on this node"
        case .docCreationFailed(let msg):
            return "Failed to create document: \(msg)"
        case .docJoinFailed(let msg):
            return "Failed to join document: \(msg)"
        case .docClosed:
            return "Document has been closed"
        case .docGetFailed(let msg):
            return "Failed to get entry: \(msg)"
        case .docSetFailed(let msg):
            return "Failed to set entry: \(msg)"
        case .docDeleteFailed(let msg):
            return "Failed to delete entry: \(msg)"
        case .docShareFailed(let msg):
            return "Failed to share document: \(msg)"
        case .contentReadFailed(let msg):
            return "Failed to read content: \(msg)"
        case .docSubscribeFailed(let msg):
            return "Failed to subscribe to document: \(msg)"
        case .authorCreationFailed(let msg):
            return "Failed to create author: \(msg)"
        case .authorImportFailed(let msg):
            return "Failed to import author: \(msg)"
        case .keychainError(let msg):
            return "Keychain error: \(msg)"
        case .blobTagFailed(let msg):
            return "Failed to tag blob: \(msg)"
        case .ticketCreationFailed(let msg):
            return "Failed to create ticket: \(msg)"
        }
    }
}
