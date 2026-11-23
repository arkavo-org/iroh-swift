import Foundation

/// MainActor-safe progress callback type for UI updates.
public typealias MainActorProgressHandler = @MainActor @Sendable (DownloadProgress) -> Void

extension IrohNode {
    /// Download bytes from a ticket with progress updates delivered strictly in order on MainActor.
    ///
    /// This is a convenience method for SwiftUI and UIKit integration where
    /// progress updates need to be handled on the main thread. All progress
    /// callbacks complete before this method returns, and no late callbacks are
    /// delivered after completion or error.
    ///
    /// - Parameters:
    ///   - ticket: The ticket string obtained from another node's `put` call.
    ///   - onProgress: Called on MainActor with progress updates during the download.
    /// - Returns: The downloaded data.
    /// - Throws: `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.getFailed` if the download fails,
    ///           `CancellationError` if the task was cancelled.
    public func getForUI(
        ticket: String,
        onProgress: @escaping MainActorProgressHandler
    ) async throws -> Data {
        try await getForUI(ticket: ticket, options: .default, onProgress: onProgress)
    }

    /// Download bytes from a ticket with options and progress updates delivered
    /// strictly in order on MainActor.
    ///
    /// - Parameters:
    ///   - ticket: The ticket string obtained from another node's `put` call.
    ///   - options: Operation options including timeout and maximum blob size.
    ///   - onProgress: Called on MainActor with progress updates during the download.
    /// - Returns: The downloaded data.
    /// - Throws: `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.timeout` if the operation times out,
    ///           `IrohError.sizeLimitExceeded` if the blob exceeds `options.maxSize`,
    ///           `IrohError.getFailed` if the download fails,
    ///           `CancellationError` if the task was cancelled.
    public func getForUI(
        ticket: String,
        options: OperationOptions,
        onProgress: @escaping MainActorProgressHandler
    ) async throws -> Data {
        let (stream, continuation) = AsyncStream.makeStream(of: DownloadProgress.self)
        let consumer = Task { @MainActor in
            for await progress in stream {
                if Task.isCancelled { break }
                onProgress(progress)
            }
        }

        do {
            let data = try await get(ticket: ticket, options: options) { progress in
                continuation.yield(progress)
            }
            continuation.finish()
            await consumer.value
            return data
        } catch {
            continuation.finish()
            consumer.cancel()
            await consumer.value
            throw error
        }
    }
}
