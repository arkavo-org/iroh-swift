import Foundation
import IrohSwiftFFI

/// Options for put/get operations.
public struct OperationOptions: Sendable {
    /// Timeout for the operation. Nil means no timeout.
    public var timeout: Duration?

    /// Default options (no timeout).
    public static let `default` = OperationOptions()

    /// Create operation options.
    ///
    /// - Parameter timeout: Timeout for the operation. Nil means no timeout.
    public init(timeout: Duration? = nil) {
        self.timeout = timeout
    }

    /// Convert timeout to milliseconds for FFI.
    var timeoutMs: UInt64 {
        guard let timeout = timeout else { return 0 }
        let components = timeout.components
        return UInt64(components.seconds * 1000) + UInt64(components.attoseconds / 1_000_000_000_000_000)
    }
}

/// Thread-safe Iroh node for blob storage and retrieval.
///
/// IrohNode is implemented as a Swift actor to ensure thread-safe access
/// and integrates with Swift's structured concurrency.
///
/// Example usage:
/// ```swift
/// let node = try await IrohNode()
///
/// // Store data and get a ticket
/// let data = "Hello, Iroh!".data(using: .utf8)!
/// let ticket = try await node.put(data)
///
/// // Later, retrieve data using the ticket
/// let retrieved = try await node.get(ticket: ticket)
///
/// // Explicitly close when done
/// try await node.close()
/// ```
public actor IrohNode {
    let handle: NodeHandleWrapper
    private var isClosed = false

    /// Create a new Iroh node with the specified configuration.
    ///
    /// - Parameter config: Configuration options. Uses defaults if not specified.
    /// - Throws: `IrohError.invalidConfiguration` if validation fails,
    ///           `IrohError.nodeCreationFailed` if the node cannot be created.
    public init(config: IrohConfig = IrohConfig()) async throws {
        // Validate configuration first
        try config.validate()

        let wrapper: NodeHandleWrapper = try await withCheckedThrowingContinuation { continuation in
            // Create the FFI config
            let storagePath = config.storagePath.path

            // Helper to create node with optional relay URL
            func createNode(pathPtr: UnsafePointer<CChar>, relayUrlPtr: UnsafePointer<CChar>?) {
                let ffiConfig = IrohNodeConfig(
                    storage_path: pathPtr,
                    relay_enabled: config.relayEnabled,
                    custom_relay_url: relayUrlPtr,
                    docs_enabled: config.docsEnabled
                )

                let box = Unmanaged.passRetained(
                    ContinuationBox<NodeHandleWrapper>(continuation)
                ).toOpaque()

                let callback = IrohNodeCreateCallback(
                    userdata: box,
                    on_success: { userdata, handlePtr in
                        let box = Unmanaged<ContinuationBox<NodeHandleWrapper>>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let wrapper = NodeHandleWrapper(pointer: handlePtr!)
                        box.continuation.resume(returning: wrapper)
                    },
                    on_failure: { userdata, errorPtr in
                        let box = Unmanaged<ContinuationBox<NodeHandleWrapper>>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let message = String(cString: errorPtr!)
                        iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                        box.continuation.resume(throwing: IrohError.nodeCreationFailed(message))
                    }
                )

                iroh_node_create(ffiConfig, callback)
            }

            storagePath.withCString { pathPtr in
                if let relayUrl = config.customRelayUrl {
                    relayUrl.withCString { relayUrlPtr in
                        createNode(pathPtr: pathPtr, relayUrlPtr: relayUrlPtr)
                    }
                } else {
                    createNode(pathPtr: pathPtr, relayUrlPtr: nil)
                }
            }
        }
        self.handle = wrapper
    }

    deinit {
        // Only destroy if not already closed
        if !isClosed {
            iroh_node_destroy(handle.pointer)
        }
    }

    /// Explicitly close the node and release resources.
    ///
    /// After calling close(), the node cannot be used for any operations.
    /// This is preferred over letting deinit handle cleanup when you need
    /// to await graceful shutdown completion.
    ///
    /// - Throws: `IrohError.closeFailed` if shutdown fails.
    public func close() async throws {
        guard !isClosed else { return }
        isClosed = true

        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            let box = Unmanaged.passRetained(
                VoidContinuationBox(continuation)
            ).toOpaque()

            let callback = IrohCloseCallback(
                userdata: box,
                on_complete: { userdata in
                    let box = Unmanaged<VoidContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    box.continuation.resume()
                },
                on_failure: { userdata, errorPtr in
                    let box = Unmanaged<VoidContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    let message = String(cString: errorPtr!)
                    iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                    box.continuation.resume(throwing: IrohError.closeFailed(message))
                }
            )

            iroh_node_close(handle.pointer, callback)
        }
    }

    /// Check if the node has been closed.
    /// - Throws: `IrohError.nodeClosed` if the node is closed.
    func ensureNotClosed() throws {
        if isClosed {
            throw IrohError.nodeClosed
        }
    }

    /// Add bytes to the blob store and return a shareable ticket.
    ///
    /// The ticket can be shared with other nodes to allow them to download
    /// the data.
    ///
    /// - Parameter data: The data to store.
    /// - Returns: A ticket string that can be used to retrieve the data.
    /// - Throws: `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.putFailed` if the operation fails,
    ///           `CancellationError` if the task was cancelled.
    public func put(_ data: Data) async throws -> String {
        try ensureNotClosed()
        try Task.checkCancellation()
        return try await withCheckedThrowingContinuation { continuation in
            data.withUnsafeBytes { buffer in
                let bytes = IrohBytes(
                    data: buffer.baseAddress?.assumingMemoryBound(to: UInt8.self),
                    len: UInt(buffer.count)
                )

                let box = Unmanaged.passRetained(
                    ContinuationBox<String>(continuation)
                ).toOpaque()

                let callback = IrohCallback(
                    userdata: box,
                    on_success: { userdata, ticketPtr in
                        let box = Unmanaged<ContinuationBox<String>>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let ticket = String(cString: ticketPtr!)
                        iroh_string_free(UnsafeMutablePointer(mutating: ticketPtr))
                        box.continuation.resume(returning: ticket)
                    },
                    on_failure: { userdata, errorPtr in
                        let box = Unmanaged<ContinuationBox<String>>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let message = String(cString: errorPtr!)
                        iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                        box.continuation.resume(throwing: IrohError.putFailed(message))
                    }
                )

                iroh_put(handle.pointer, bytes, callback)
            }
        }
    }

    /// Add bytes to the blob store with options (e.g., timeout).
    ///
    /// - Parameters:
    ///   - data: The data to store.
    ///   - options: Operation options including timeout.
    /// - Returns: A ticket string that can be used to retrieve the data.
    /// - Throws: `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.timeout` if the operation times out,
    ///           `IrohError.putFailed` if the operation fails.
    public func put(_ data: Data, options: OperationOptions) async throws -> String {
        try ensureNotClosed()
        try Task.checkCancellation()
        return try await withCheckedThrowingContinuation { continuation in
            data.withUnsafeBytes { buffer in
                let bytes = IrohBytes(
                    data: buffer.baseAddress?.assumingMemoryBound(to: UInt8.self),
                    len: UInt(buffer.count)
                )

                let ffiOptions = IrohOperationOptions(timeout_ms: options.timeoutMs)

                let box = Unmanaged.passRetained(
                    ContinuationBox<String>(continuation)
                ).toOpaque()

                let callback = IrohCallback(
                    userdata: box,
                    on_success: { userdata, ticketPtr in
                        let box = Unmanaged<ContinuationBox<String>>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let ticket = String(cString: ticketPtr!)
                        iroh_string_free(UnsafeMutablePointer(mutating: ticketPtr))
                        box.continuation.resume(returning: ticket)
                    },
                    on_failure: { userdata, errorPtr in
                        let box = Unmanaged<ContinuationBox<String>>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let message = String(cString: errorPtr!)
                        iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                        // Check if it's a timeout error
                        if message.contains("timed out") {
                            box.continuation.resume(throwing: IrohError.timeout)
                        } else {
                            box.continuation.resume(throwing: IrohError.putFailed(message))
                        }
                    }
                )

                iroh_put_with_options(handle.pointer, bytes, ffiOptions, callback)
            }
        }
    }

    /// Download bytes from a ticket.
    ///
    /// This fetches the blob from the remote peer specified in the ticket.
    ///
    /// - Parameter ticket: The ticket string obtained from another node's `put` call.
    /// - Returns: The downloaded data.
    /// - Throws: `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.getFailed` if the download fails,
    ///           `CancellationError` if the task was cancelled.
    public func get(ticket: String) async throws -> Data {
        try ensureNotClosed()
        try Task.checkCancellation()
        return try await withCheckedThrowingContinuation { continuation in
            ticket.withCString { ticketPtr in
                let box = Unmanaged.passRetained(
                    ContinuationBox<Data>(continuation)
                ).toOpaque()

                let callback = IrohGetCallback(
                    userdata: box,
                    on_success: { userdata, ownedBytes in
                        let box = Unmanaged<ContinuationBox<Data>>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let data = Data(bytes: ownedBytes.data, count: Int(ownedBytes.len))
                        iroh_bytes_free(ownedBytes)
                        box.continuation.resume(returning: data)
                    },
                    on_failure: { userdata, errorPtr in
                        let box = Unmanaged<ContinuationBox<Data>>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let message = String(cString: errorPtr!)
                        iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                        box.continuation.resume(throwing: IrohError.getFailed(message))
                    }
                )

                iroh_get(handle.pointer, ticketPtr, callback)
            }
        }
    }

    /// Download bytes from a ticket with options (e.g., timeout).
    ///
    /// - Parameters:
    ///   - ticket: The ticket string obtained from another node's `put` call.
    ///   - options: Operation options including timeout.
    /// - Returns: The downloaded data.
    /// - Throws: `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.timeout` if the operation times out,
    ///           `IrohError.getFailed` if the download fails.
    public func get(ticket: String, options: OperationOptions) async throws -> Data {
        try ensureNotClosed()
        try Task.checkCancellation()
        return try await withCheckedThrowingContinuation { continuation in
            ticket.withCString { ticketPtr in
                let ffiOptions = IrohOperationOptions(timeout_ms: options.timeoutMs)

                let box = Unmanaged.passRetained(
                    ContinuationBox<Data>(continuation)
                ).toOpaque()

                let callback = IrohGetCallback(
                    userdata: box,
                    on_success: { userdata, ownedBytes in
                        let box = Unmanaged<ContinuationBox<Data>>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let data = Data(bytes: ownedBytes.data, count: Int(ownedBytes.len))
                        iroh_bytes_free(ownedBytes)
                        box.continuation.resume(returning: data)
                    },
                    on_failure: { userdata, errorPtr in
                        let box = Unmanaged<ContinuationBox<Data>>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let message = String(cString: errorPtr!)
                        iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                        // Check if it's a timeout error
                        if message.contains("timed out") {
                            box.continuation.resume(throwing: IrohError.timeout)
                        } else {
                            box.continuation.resume(throwing: IrohError.getFailed(message))
                        }
                    }
                )

                iroh_get_with_options(handle.pointer, ticketPtr, ffiOptions, callback)
            }
        }
    }
}

// MARK: - Internal Helpers

/// Box for passing Swift continuations through FFI callbacks.
private final class ContinuationBox<T>: @unchecked Sendable {
    let continuation: CheckedContinuation<T, Error>

    init(_ continuation: CheckedContinuation<T, Error>) {
        self.continuation = continuation
    }
}

/// Box for Void continuations (used by close callback).
private final class VoidContinuationBox: @unchecked Sendable {
    let continuation: CheckedContinuation<Void, Error>

    init(_ continuation: CheckedContinuation<Void, Error>) {
        self.continuation = continuation
    }
}

/// Sendable wrapper for the node handle pointer.
/// This is safe because the handle is thread-safe in Rust and only
/// accessed through actor isolation in Swift.
struct NodeHandleWrapper: @unchecked Sendable {
    let pointer: UnsafeMutablePointer<IrohNodeHandle>
}
