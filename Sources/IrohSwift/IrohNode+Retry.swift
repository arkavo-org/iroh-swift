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
    /// - Throws: `IrohError.maxRetriesExceeded` if all attempts fail.
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
    /// - Throws: `IrohError.maxRetriesExceeded` if all attempts fail.
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
        var lastError: (any Error)?
        var currentDelay = initialDelay

        for attempt in 1...maxAttempts {
            try Task.checkCancellation()

            do {
                return try await operation()
            } catch {
                lastError = error

                // Don't sleep after the last attempt
                if attempt < maxAttempts {
                    try await Task.sleep(for: currentDelay)
                    // Exponential backoff: double the delay each time
                    currentDelay = Duration.seconds(currentDelay.components.seconds * 2)
                }
            }
        }

        throw IrohError.maxRetriesExceeded(attempts: maxAttempts, lastError: lastError!)
    }
}
