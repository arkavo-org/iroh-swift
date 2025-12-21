import Foundation
import IrohSwiftFFI

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
/// ```
public actor IrohNode {
    let handle: NodeHandleWrapper

    /// Create a new Iroh node with the specified configuration.
    ///
    /// - Parameter config: Configuration options. Uses defaults if not specified.
    /// - Throws: `IrohError.nodeCreationFailed` if the node cannot be created.
    public init(config: IrohConfig = IrohConfig()) async throws {
        let wrapper: NodeHandleWrapper = try await withCheckedThrowingContinuation { continuation in
            // Create the FFI config
            let storagePath = config.storagePath.path
            storagePath.withCString { pathPtr in
                let ffiConfig = IrohNodeConfig(
                    storage_path: pathPtr,
                    relay_enabled: config.relayEnabled
                )

                // Create callback wrapper
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
        }
        self.handle = wrapper
    }

    deinit {
        iroh_node_destroy(handle.pointer)
    }

    /// Add bytes to the blob store and return a shareable ticket.
    ///
    /// The ticket can be shared with other nodes to allow them to download
    /// the data.
    ///
    /// - Parameter data: The data to store.
    /// - Returns: A ticket string that can be used to retrieve the data.
    /// - Throws: `IrohError.putFailed` if the operation fails.
    public func put(_ data: Data) async throws -> String {
        try await withCheckedThrowingContinuation { continuation in
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

    /// Download bytes from a ticket.
    ///
    /// This fetches the blob from the remote peer specified in the ticket.
    ///
    /// - Parameter ticket: The ticket string obtained from another node's `put` call.
    /// - Returns: The downloaded data.
    /// - Throws: `IrohError.getFailed` if the download fails.
    public func get(ticket: String) async throws -> Data {
        try await withCheckedThrowingContinuation { continuation in
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
}

// MARK: - Internal Helpers

/// Box for passing Swift continuations through FFI callbacks.
private final class ContinuationBox<T>: @unchecked Sendable {
    let continuation: CheckedContinuation<T, Error>

    init(_ continuation: CheckedContinuation<T, Error>) {
        self.continuation = continuation
    }
}

/// Sendable wrapper for the node handle pointer.
/// This is safe because the handle is thread-safe in Rust and only
/// accessed through actor isolation in Swift.
struct NodeHandleWrapper: @unchecked Sendable {
    let pointer: UnsafeMutablePointer<IrohNodeHandle>
}
