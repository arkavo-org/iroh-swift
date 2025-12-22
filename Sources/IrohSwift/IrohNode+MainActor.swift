import Foundation

/// MainActor-safe progress callback type for UI updates.
public typealias MainActorProgressHandler = @MainActor @Sendable (DownloadProgress) -> Void

extension IrohNode {
    /// Download bytes from a ticket with progress updates delivered on MainActor.
    ///
    /// This is a convenience method for SwiftUI and UIKit integration where
    /// progress updates need to be handled on the main thread.
    ///
    /// Example:
    /// ```swift
    /// @State private var progress: Double = 0
    ///
    /// func download() async throws {
    ///     let data = try await node.getForUI(ticket: ticket) { progress in
    ///         self.progress = progress.fraction ?? 0
    ///     }
    /// }
    /// ```
    ///
    /// - Parameters:
    ///   - ticket: The ticket string obtained from another node's `put` call.
    ///   - onProgress: Called on MainActor with progress updates during the download.
    /// - Returns: The downloaded data.
    /// - Throws: `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.getFailed` if the download fails.
    public func getForUI(
        ticket: String,
        onProgress: @escaping MainActorProgressHandler
    ) async throws -> Data {
        try await get(ticket: ticket) { progress in
            Task { @MainActor in
                onProgress(progress)
            }
        }
    }

    /// Download bytes from a ticket with options and progress updates on MainActor.
    ///
    /// - Parameters:
    ///   - ticket: The ticket string obtained from another node's `put` call.
    ///   - options: Operation options including timeout.
    ///   - onProgress: Called on MainActor with progress updates during the download.
    /// - Returns: The downloaded data.
    /// - Throws: `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.timeout` if the operation times out,
    ///           `IrohError.getFailed` if the download fails.
    public func getForUI(
        ticket: String,
        options: OperationOptions,
        onProgress: @escaping MainActorProgressHandler
    ) async throws -> Data {
        // For timeout variant, we use the options-based get which doesn't have progress
        // but wraps it with the MainActor callback pattern
        try ensureNotClosed()
        try Task.checkCancellation()

        // Initial progress notification
        Task { @MainActor in
            onProgress(DownloadProgress(downloaded: 0, total: 0))
        }

        let data = try await get(ticket: ticket, options: options)

        // Final progress notification
        Task { @MainActor in
            onProgress(DownloadProgress(downloaded: UInt64(data.count), total: UInt64(data.count)))
        }

        return data
    }
}
