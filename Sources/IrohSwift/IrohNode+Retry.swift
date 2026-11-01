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
    ///   `IrohError.invalidConfiguration` if `maxAttempts` is less than 1.
    public func putWithRetry(
        _ data: Data,
        maxAttempts: Int = 3,
        initialDelay: Duration = .seconds(1)
    ) async throws -> String {
        try await withRetry(maxAttempts: maxAttempts, initialDelay: initialDelay) {
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
    ///   `IrohError.invalidConfiguration` if `maxAttempts` is less than 1.
    public func getWithRetry(
        ticket: String,
        maxAttempts: Int = 3,
        initialDelay: Duration = .seconds(1)
    ) async throws -> Data {
        try await withRetry(maxAttempts: maxAttempts, initialDelay: initialDelay) {
            try await self.get(ticket: ticket)
        }
    }

    /// Generic retry helper with exponential backoff.
    private func withRetry<T>(
        maxAttempts: Int,
        initialDelay: Duration,
        operation: @Sendable () async throws -> T
    ) async throws -> T {
        // `1...maxAttempts` traps for values below 1.
        guard maxAttempts >= 1 else {
            throw IrohError.invalidConfiguration("maxAttempts must be at least 1, got \(maxAttempts)")
        }

        var currentDelay = initialDelay

        for attempt in 1...maxAttempts {
            try Task.checkCancellation()

            do {
                return try await operation()
            } catch {
                // The last attempt reports its own error, so none has to be stored
                guard attempt < maxAttempts else {
                    throw IrohError.maxRetriesExceeded(attempts: maxAttempts, lastError: error)
                }
                try await Task.sleep(for: currentDelay)
                // Exponential backoff: double the delay each time
                currentDelay *= 2
            }
        }

        // Unreachable: the final attempt either returns or throws above.
        preconditionFailure("withRetry loop exited without a result")
    }
}
