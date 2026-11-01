import Foundation

extension IrohNode {
    /// Store data with automatic retry on failure.
    ///
    /// Uses exponential backoff between retry attempts.
    ///
    /// - Parameters:
    ///   - data: The data to store.
    ///   - maxAttempts: Maximum number of attempts. Defaults to 3.
    ///   - initialDelay: Delay before first retry. Defaults to 1 second.
    /// - Returns: A ticket string for retrieving the data.
    /// - Throws: `IrohError.maxRetriesExceeded` if all attempts fail,
    ///   `IrohError.invalidConfiguration` if `maxAttempts` is less than 1,
    ///   `IrohError.nodeClosed` (not retried) if the node is closed,
    ///   `CancellationError` if the task is cancelled.
    public func putWithRetry(
        _ data: Data,
        maxAttempts: Int = 3,
        initialDelay: Duration = .seconds(1)
    ) async throws -> String {
        try await Self.withRetry(maxAttempts: maxAttempts, initialDelay: initialDelay) {
            try await self.put(data)
        }
    }

    /// Retrieve data with automatic retry on failure.
    ///
    /// Uses exponential backoff between retry attempts.
    ///
    /// - Parameters:
    ///   - ticket: The ticket string from a `put` operation.
    ///   - maxAttempts: Maximum number of attempts. Defaults to 3.
    ///   - initialDelay: Delay before first retry. Defaults to 1 second.
    /// - Returns: The downloaded data.
    /// - Throws: `IrohError.maxRetriesExceeded` if all attempts fail,
    ///   `IrohError.invalidConfiguration` if `maxAttempts` is less than 1,
    ///   `IrohError.nodeClosed` (not retried) if the node is closed,
    ///   `CancellationError` if the task is cancelled.
    public func getWithRetry(
        ticket: String,
        maxAttempts: Int = 3,
        initialDelay: Duration = .seconds(1)
    ) async throws -> Data {
        try await Self.withRetry(maxAttempts: maxAttempts, initialDelay: initialDelay) {
            try await self.get(ticket: ticket)
        }
    }

    /// Generic retry helper with exponential backoff.
    ///
    /// Cancellation and `IrohError.nodeClosed` are thrown as they are, on any
    /// attempt: no retry can fix them. Other errors are retried, and the last
    /// one is reported in `IrohError.maxRetriesExceeded`.
    static func withRetry<T: Sendable>(
        maxAttempts: Int,
        initialDelay: Duration,
        operation: @Sendable () async throws -> T
    ) async throws -> T {
        // `1..<maxAttempts` traps for values below 1.
        guard maxAttempts >= 1 else {
            throw IrohError.invalidConfiguration("maxAttempts must be at least 1, got \(maxAttempts)")
        }

        var currentDelay = initialDelay
        for _ in 1..<maxAttempts {
            try Task.checkCancellation()
            do {
                return try await operation()
            } catch let error where isPermanent(error) {
                throw error
            } catch {
                try await Task.sleep(for: currentDelay)
                currentDelay *= 2
            }
        }

        try Task.checkCancellation()
        do {
            return try await operation()
        } catch let error where isPermanent(error) {
            throw error
        } catch {
            throw IrohError.maxRetriesExceeded(attempts: maxAttempts, lastError: error)
        }
    }

    /// Errors a retry cannot fix.
    private static func isPermanent(_ error: Error) -> Bool {
        if error is CancellationError { return true }
        if case IrohError.nodeClosed = error { return true }
        return false
    }
}
