import Foundation
import IrohSwiftFFI

extension IrohNode {
    /// Push bytes directly to a remote node identified by its node ID.
    ///
    /// Adds the data to the local store, connects to the remote node,
    /// and pushes the blob. Returns the blob hash as a hex string on success.
    ///
    /// The remote node must accept pushes (an `IrohNode` created with
    /// `acceptsPushes: true`, or a server that enables them). A node that refuses
    /// pushes stores nothing, and the pusher is not reliably told: a small blob
    /// can still return its hash; a larger one fails with `IrohError.pushFailed`.
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
    /// Like `pushToNode(nodeId:relayUrl:directAddrs:data:)`, the remote node must
    /// accept pushes.
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

extension IrohNode {
    /// Push a file directly to a remote node without loading it into memory.
    ///
    /// The file is streamed into this node's blob store (copied, so the store
    /// needs free space for it; the original file is not modified or kept open
    /// afterwards), then pushed to the remote node exactly like
    /// `pushToNode(nodeId:relayUrl:directAddrs:data:)`, so the remote node must
    /// accept pushes. Works on client-only nodes (`servesBlobs: false`). Like the
    /// `Data` variant, the blob stays in this node's store afterwards.
    ///
    /// Cancelling the calling task aborts the import or push and throws
    /// `CancellationError`.
    ///
    /// - Parameters:
    ///   - nodeId: The remote node ID as a hex string.
    ///   - relayUrl: Optional relay URL to assist with connectivity.
    ///   - directAddrs: Optional list of direct addresses for the remote node.
    ///   - fileURL: File URL of the file to push.
    /// - Returns: The blob hash (BLAKE3 of the file contents) as 64 lowercase hex characters.
    /// - Throws: `IrohError.nodeClosed` if the node is or gets closed,
    ///           `IrohError.pushFailed` if the import or push fails,
    ///           `CancellationError` if the task was cancelled.
    public func pushToNode(
        nodeId: String,
        relayUrl: String? = nil,
        directAddrs: [String] = [],
        fileURL: URL
    ) async throws -> String {
        try ensureNotClosed()
        try Task.checkCancellation()
        guard fileURL.isFileURL else {
            throw IrohError.pushFailed("fileURL must be a file URL: \(fileURL)")
        }

        let token = CancelTokenBox()
        let filePath = fileURL.standardizedFileURL.path
        let handle = self.handle

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<String, Error>) in
                let userdata = Unmanaged.passRetained(
                    ContinuationBox<String>(continuation)
                ).toOpaque()

                let callback = IrohPushFileCallback(
                    userdata: userdata,
                    on_success: { userdata, hashPtr in
                        let box = Unmanaged<ContinuationBox<String>>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        box.continuation.resume(returning: takeFFIString(hashPtr))
                    },
                    on_failure: { userdata, code, errorPtr in
                        let box = Unmanaged<ContinuationBox<String>>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let message = takeFFIString(errorPtr)
                        let error: Error
                        switch code {
                        case IrohOperationErrorCode_Cancelled:
                            error = CancellationError()
                        case IrohOperationErrorCode_NodeShutDown:
                            error = IrohError.nodeClosed
                        default:
                            error = IrohError.pushFailed(message)
                        }
                        box.continuation.resume(throwing: error)
                    }
                )

                nodeId.withCString { nodeIdPtr in
                    filePath.withCString { filePtr in
                        withOptionalCString(relayUrl) { relayPtr in
                            withCStringArray(directAddrs) { addrs, count in
                                iroh_push_file_to_node(
                                    handle.pointer,
                                    nodeIdPtr,
                                    relayPtr,
                                    addrs,
                                    UInt(count),
                                    filePtr,
                                    token.pointer,
                                    callback
                                )
                            }
                        }
                    }
                }
            }
        } onCancel: {
            token.cancel()
        }
    }
}
