import Foundation

/// Progress callback type for download operations.
public typealias ProgressHandler = @Sendable (DownloadProgress) -> Void

extension IrohNode {
    /// Download bytes from a ticket with progress reporting.
    ///
    /// - Parameters:
    ///   - ticket: The ticket string obtained from another node's `put` call.
    ///   - onProgress: Called with progress updates during the download.
    /// - Returns: The downloaded data.
    /// - Throws: `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.getFailed` if the download fails,
    ///           `CancellationError` if the task was cancelled.
    public func get(
        ticket: String,
        onProgress: @escaping ProgressHandler
    ) async throws -> Data {
        try await getBytes(ticket: ticket, options: .default, onProgress: onProgress)
    }

    /// Download bytes from a ticket with options (e.g., timeout, maxSize) and progress reporting.
    ///
    /// - Parameters:
    ///   - ticket: The ticket string obtained from another node's `put` call.
    ///   - options: Operation options including timeout and maximum blob size.
    ///   - onProgress: Called with progress updates during the download.
    /// - Returns: The downloaded data.
    /// - Throws: `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.timeout` if the operation times out,
    ///           `IrohError.sizeLimitExceeded` if the blob exceeds `options.maxSize`,
    ///           `IrohError.getFailed` if the download fails,
    ///           `CancellationError` if the task was cancelled.
    public func get(
        ticket: String,
        options: OperationOptions,
        onProgress: @escaping ProgressHandler
    ) async throws -> Data {
        try await getBytes(ticket: ticket, options: options, onProgress: onProgress)
    }
}
