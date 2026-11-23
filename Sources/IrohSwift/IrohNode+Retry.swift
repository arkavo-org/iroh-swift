import Foundation

extension IrohNode {
    /// Default cap for exponential backoff delays.
    static let defaultMaxRetryDelay: Duration = .seconds(60)

    /// Store data with automatic retry on failure.
    ///
    /// Uses capped exponential backoff between retry attempts.
    ///
    /// - Parameters:
    ///   - data: The data to store.
    ///   - maxAttempts: Maximum number of attempts. Defaults to 3.
    ///   - initialDelay: Delay before first retry. Defaults to 1 second.
    ///   - maxDelay: Maximum backoff delay between retries. Defaults to 60 seconds.
    /// - Returns: A ticket string for retrieving the data.
    /// - Throws: `IrohError.maxRetriesExceeded` if all attempts fail,
    ///   `IrohError.invalidConfiguration` if `maxAttempts` is less than 1,
    ///   `IrohError.nodeClosed` (not retried) if the node is closed,
    ///   `CancellationError` if the task is cancelled.
    public func putWithRetry(
        _ data: Data,
        maxAttempts: Int = 3,
        initialDelay: Duration = .seconds(1),
        maxDelay: Duration = .seconds(60),
        options: OperationOptions = .default
    ) async throws -> String {
        try await Self.withRetry(
            maxAttempts: maxAttempts,
            initialDelay: initialDelay,
            maxDelay: maxDelay
        ) {
            try await self.put(data, options: options)
        }
    }

    /// Retrieve data with automatic retry on failure.
    ///
    /// Uses capped exponential backoff between retry attempts.
    ///
    /// - Parameters:
    ///   - ticket: The ticket string from a `put` operation.
    ///   - maxAttempts: Maximum number of attempts. Defaults to 3.
    ///   - initialDelay: Delay before first retry. Defaults to 1 second.
    ///   - maxDelay: Maximum backoff delay between retries. Defaults to 60 seconds.
    ///   - options: Operation options including timeout and maximum blob size.
    /// - Returns: The downloaded data.
    /// - Throws: `IrohError.maxRetriesExceeded` if all attempts fail,
    ///   `IrohError.invalidConfiguration` if `maxAttempts` is less than 1,
    ///   `IrohError.nodeClosed` or `IrohError.sizeLimitExceeded` (not retried),
    ///   `CancellationError` if the task is cancelled.
    public func getWithRetry(
        ticket: String,
        maxAttempts: Int = 3,
        initialDelay: Duration = .seconds(1),
        maxDelay: Duration = .seconds(60),
        options: OperationOptions = .default
    ) async throws -> Data {
        try await Self.withRetry(
            maxAttempts: maxAttempts,
            initialDelay: initialDelay,
            maxDelay: maxDelay
        ) {
            try await self.get(ticket: ticket, options: options)
        }
    }

    /// Double `delay` without overflowing `Duration`, capped at `max(maxDelay, delay)`.
    static func nextBackoffDelay(after delay: Duration, maxDelay: Duration) -> Duration {
        let clampedDelay = max(delay, .zero)
        let cap = max(max(maxDelay, .zero), clampedDelay)
        let halfCap = cap / 2
        if clampedDelay >= halfCap {
            return cap
        }
        return clampedDelay * 2
    }

    /// Generic retry helper with capped exponential backoff.
    ///
    /// Cancellation and `IrohError.nodeClosed` are thrown as they are, on any
    /// attempt: no retry can fix them. Other errors are retried, and the last
    /// one is reported in `IrohError.maxRetriesExceeded`.
    static func withRetry<T: Sendable>(
        maxAttempts: Int,
        initialDelay: Duration,
        maxDelay: Duration = defaultMaxRetryDelay,
        operation: @Sendable () async throws -> T
    ) async throws -> T {
        // `1..<maxAttempts` traps for values below 1.
        guard maxAttempts >= 1 else {
            throw IrohError.invalidConfiguration("maxAttempts must be at least 1, got \(maxAttempts)")
        }

        var currentDelay = max(initialDelay, .zero)
        for _ in 1..<maxAttempts {
            try Task.checkCancellation()
            do {
                return try await operation()
            } catch let error where isPermanent(error) {
                throw error
            } catch {
                try await Task.sleep(for: currentDelay)
                currentDelay = nextBackoffDelay(after: currentDelay, maxDelay: maxDelay)
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
        if case IrohError.sizeLimitExceeded = error { return true }
        return false
    }
}
