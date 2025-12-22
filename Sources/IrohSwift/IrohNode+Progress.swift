import Foundation
import IrohSwiftFFI

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
    ///           `IrohError.getFailed` if the download fails.
    public func get(
        ticket: String,
        onProgress: @escaping ProgressHandler
    ) async throws -> Data {
        try ensureNotClosed()
        try Task.checkCancellation()
        return try await withCheckedThrowingContinuation { continuation in
            ticket.withCString { ticketPtr in
                let context = ProgressContext(
                    continuation: continuation,
                    onProgress: onProgress
                )
                let box = Unmanaged.passRetained(context).toOpaque()

                let callback = IrohGetProgressCallback(
                    userdata: box,
                    on_progress: { userdata, progress in
                        let ctx = Unmanaged<ProgressContext>
                            .fromOpaque(userdata!)
                            .takeUnretainedValue()
                        let swiftProgress = DownloadProgress(
                            downloaded: progress.downloaded,
                            total: progress.total
                        )
                        ctx.onProgress(swiftProgress)
                    },
                    on_success: { userdata, ownedBytes in
                        let ctx = Unmanaged<ProgressContext>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let data = Data(bytes: ownedBytes.data, count: Int(ownedBytes.len))
                        iroh_bytes_free(ownedBytes)
                        ctx.continuation.resume(returning: data)
                    },
                    on_failure: { userdata, errorPtr in
                        let ctx = Unmanaged<ProgressContext>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let message = String(cString: errorPtr!)
                        iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                        ctx.continuation.resume(throwing: IrohError.getFailed(message))
                    }
                )

                iroh_get_with_progress(handle.pointer, ticketPtr, callback)
            }
        }
    }
}

// MARK: - Internal Helpers

/// Context for progress callbacks, holding both the continuation and progress handler.
private final class ProgressContext: @unchecked Sendable {
    let continuation: CheckedContinuation<Data, Error>
    let onProgress: ProgressHandler

    init(
        continuation: CheckedContinuation<Data, Error>,
        onProgress: @escaping ProgressHandler
    ) {
        self.continuation = continuation
        self.onProgress = onProgress
    }
}
