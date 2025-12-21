import Foundation
import os

/// Logger for Iroh operations.
///
/// Uses Apple's unified logging system (os.Logger).
/// Subsystem: "org.arkavo.iroh", Category: "node"
enum IrohLogger {
    static let node = Logger(subsystem: "org.arkavo.iroh", category: "node")
}

extension IrohNode {
    /// Store data with logging.
    ///
    /// Logs the operation start, success with ticket prefix, or failure.
    ///
    /// - Parameter data: The data to store.
    /// - Returns: A ticket string for retrieving the data.
    /// - Throws: `IrohError.putFailed` if the operation fails.
    public func putWithLogging(_ data: Data) async throws -> String {
        try Task.checkCancellation()

        IrohLogger.node.info("put: starting, size=\(data.count) bytes")

        do {
            let ticket = try await put(data)
            // Log only ticket prefix to avoid exposing full ticket in logs
            let ticketPrefix = String(ticket.prefix(20))
            IrohLogger.node.info("put: success, ticket=\(ticketPrefix, privacy: .public)...")
            return ticket
        } catch {
            IrohLogger.node.error("put: failed, error=\(error.localizedDescription, privacy: .public)")
            throw error
        }
    }

    /// Retrieve data with logging.
    ///
    /// Logs the operation start, success with data size, or failure.
    ///
    /// - Parameter ticket: The ticket string from a `put` operation.
    /// - Returns: The downloaded data.
    /// - Throws: `IrohError.getFailed` if the operation fails.
    public func getWithLogging(ticket: String) async throws -> Data {
        try Task.checkCancellation()

        let ticketPrefix = String(ticket.prefix(20))
        IrohLogger.node.info("get: starting, ticket=\(ticketPrefix, privacy: .public)...")

        do {
            let data = try await get(ticket: ticket)
            IrohLogger.node.info("get: success, size=\(data.count) bytes")
            return data
        } catch {
            IrohLogger.node.error("get: failed, error=\(error.localizedDescription, privacy: .public)")
            throw error
        }
    }
}
