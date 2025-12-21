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
        }
    }
}
