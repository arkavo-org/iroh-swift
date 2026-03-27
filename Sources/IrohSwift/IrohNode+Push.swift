import Foundation
import IrohSwiftFFI

extension IrohNode {
    /// Push bytes directly to a remote node identified by its node ID.
    ///
    /// Adds the data to the local store, connects to the remote node,
    /// and pushes the blob. Returns the blob hash as a hex string on success.
    ///
    /// - Parameters:
    ///   - nodeId: The remote node ID as a hex string.
    ///   - relayUrl: Optional relay URL to assist with connectivity.
    ///   - directAddrs: Optional list of direct addresses for the remote node.
    ///   - data: The data to push.
    /// - Returns: The blob hash as a hex string.
    /// - Throws: `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.pushFailed` if the push fails,
    ///           `CancellationError` if the task was cancelled.
    public func pushToNode(
        nodeId: String,
        relayUrl: String? = nil,
        directAddrs: [String] = [],
        data: Data
    ) async throws -> String {
        try ensureNotClosed()
        try Task.checkCancellation()
        return try await withCheckedThrowingContinuation { continuation in
            nodeId.withCString { nodeIdPtr in
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
                        on_success: { userdata, hashPtr in
                            let box = Unmanaged<ContinuationBox<String>>
                                .fromOpaque(userdata!)
                                .takeRetainedValue()
                            let hash = String(cString: hashPtr!)
                            iroh_string_free(UnsafeMutablePointer(mutating: hashPtr))
                            box.continuation.resume(returning: hash)
                        },
                        on_failure: { userdata, errorPtr in
                            let box = Unmanaged<ContinuationBox<String>>
                                .fromOpaque(userdata!)
                                .takeRetainedValue()
                            let message = String(cString: errorPtr!)
                            iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                            box.continuation.resume(throwing: IrohError.pushFailed(message))
                        }
                    )

                    if directAddrs.isEmpty {
                        if let relayUrl = relayUrl {
                            relayUrl.withCString { relayUrlPtr in
                                iroh_push_to_node(
                                    handle.pointer,
                                    nodeIdPtr,
                                    relayUrlPtr,
                                    nil,
                                    0,
                                    bytes,
                                    callback
                                )
                            }
                        } else {
                            iroh_push_to_node(
                                handle.pointer,
                                nodeIdPtr,
                                nil,
                                nil,
                                0,
                                bytes,
                                callback
                            )
                        }
                    } else {
                        let cAddrs = directAddrs.map { strdup($0) }
                        defer { cAddrs.forEach { free($0) } }
                        let constAddrs = cAddrs.map { UnsafePointer($0) }
                        constAddrs.withUnsafeBufferPointer { addrBuffer in
                            if let relayUrl = relayUrl {
                                relayUrl.withCString { relayUrlPtr in
                                    iroh_push_to_node(
                                        handle.pointer,
                                        nodeIdPtr,
                                        relayUrlPtr,
                                        addrBuffer.baseAddress,
                                        UInt(directAddrs.count),
                                        bytes,
                                        callback
                                    )
                                }
                            } else {
                                iroh_push_to_node(
                                    handle.pointer,
                                    nodeIdPtr,
                                    nil,
                                    addrBuffer.baseAddress,
                                    UInt(directAddrs.count),
                                    bytes,
                                    callback
                                )
                            }
                        }
                    }
                }
            }
        }
    }

    /// Push bytes directly to a remote node with options (e.g., timeout).
    ///
    /// - Parameters:
    ///   - nodeId: The remote node ID as a hex string.
    ///   - relayUrl: Optional relay URL to assist with connectivity.
    ///   - directAddrs: Optional list of direct addresses for the remote node.
    ///   - data: The data to push.
    ///   - options: Operation options including timeout.
    /// - Returns: The blob hash as a hex string.
    /// - Throws: `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.timeout` if the operation times out,
    ///           `IrohError.pushFailed` if the push fails.
    public func pushToNode(
        nodeId: String,
        relayUrl: String? = nil,
        directAddrs: [String] = [],
        data: Data,
        options: OperationOptions
    ) async throws -> String {
        try ensureNotClosed()
        try Task.checkCancellation()
        return try await withCheckedThrowingContinuation { continuation in
            nodeId.withCString { nodeIdPtr in
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
                        on_success: { userdata, hashPtr in
                            let box = Unmanaged<ContinuationBox<String>>
                                .fromOpaque(userdata!)
                                .takeRetainedValue()
                            let hash = String(cString: hashPtr!)
                            iroh_string_free(UnsafeMutablePointer(mutating: hashPtr))
                            box.continuation.resume(returning: hash)
                        },
                        on_failure: { userdata, errorPtr in
                            let box = Unmanaged<ContinuationBox<String>>
                                .fromOpaque(userdata!)
                                .takeRetainedValue()
                            let message = String(cString: errorPtr!)
                            iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                            if message.contains("timed out") {
                                box.continuation.resume(throwing: IrohError.timeout)
                            } else {
                                box.continuation.resume(throwing: IrohError.pushFailed(message))
                            }
                        }
                    )

                    if directAddrs.isEmpty {
                        if let relayUrl = relayUrl {
                            relayUrl.withCString { relayUrlPtr in
                                iroh_push_to_node_with_options(
                                    handle.pointer,
                                    nodeIdPtr,
                                    relayUrlPtr,
                                    nil,
                                    0,
                                    bytes,
                                    ffiOptions,
                                    callback
                                )
                            }
                        } else {
                            iroh_push_to_node_with_options(
                                handle.pointer,
                                nodeIdPtr,
                                nil,
                                nil,
                                0,
                                bytes,
                                ffiOptions,
                                callback
                            )
                        }
                    } else {
                        let cAddrs = directAddrs.map { strdup($0) }
                        defer { cAddrs.forEach { free($0) } }
                        let constAddrs = cAddrs.map { UnsafePointer($0) }
                        constAddrs.withUnsafeBufferPointer { addrBuffer in
                            if let relayUrl = relayUrl {
                                relayUrl.withCString { relayUrlPtr in
                                    iroh_push_to_node_with_options(
                                        handle.pointer,
                                        nodeIdPtr,
                                        relayUrlPtr,
                                        addrBuffer.baseAddress,
                                        UInt(directAddrs.count),
                                        bytes,
                                        ffiOptions,
                                        callback
                                    )
                                }
                            } else {
                                iroh_push_to_node_with_options(
                                    handle.pointer,
                                    nodeIdPtr,
                                    nil,
                                    addrBuffer.baseAddress,
                                    UInt(directAddrs.count),
                                    bytes,
                                    ffiOptions,
                                    callback
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}
