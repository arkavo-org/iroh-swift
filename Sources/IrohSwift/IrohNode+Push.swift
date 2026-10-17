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
    ///   - directAddrs: Optional direct addresses of the remote node: `"ip:port"`
    ///     (`"[ipv6]:port"` for IPv6) or `"host:port"`. A hostname resolves to
    ///     all its A and AAAA records, through public DNS-over-HTTPS resolvers
    ///     first and the device's system resolver only if none answers; a
    ///     literal IP is used without any DNS. DNS cannot impersonate the node,
    ///     since the QUIC handshake checks `nodeId`.
    ///   - data: The data to push.
    /// - Returns: The blob hash as a hex string.
    /// - Throws: `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.pushFailed` if the push fails, including a malformed
    ///           direct address or hostnames that resolve to nothing with no relay
    ///           URL or other address to fall back on (unless `IrohConfig.dhtLookup`
    ///           is on: then the push dials by `nodeId` alone),
    ///           `CancellationError` if the task was cancelled (also during hostname
    ///           resolution; nothing is sent after a cancel).
    public func pushToNode(
        nodeId: String,
        relayUrl: String? = nil,
        directAddrs: [String] = [],
        data: Data
    ) async throws -> String {
        try await pushBytes(
            nodeId: nodeId, relayUrl: relayUrl, directAddrs: directAddrs, data: data, timeoutMs: 0
        )
    }

    /// Push bytes directly to a remote node with options (e.g., timeout).
    ///
    /// Like `pushToNode(nodeId:relayUrl:directAddrs:data:)`, the remote node must
    /// accept pushes.
    ///
    /// - Parameters:
    ///   - nodeId: The remote node ID as a hex string.
    ///   - relayUrl: Optional relay URL to assist with connectivity.
    ///   - directAddrs: Optional direct addresses of the remote node: `"ip:port"`
    ///     (`"[ipv6]:port"` for IPv6) or `"host:port"`. A hostname resolves to
    ///     all its A and AAAA records, through public DNS-over-HTTPS resolvers
    ///     first and the device's system resolver only if none answers; a
    ///     literal IP is used without any DNS. DNS cannot impersonate the node,
    ///     since the QUIC handshake checks `nodeId`.
    ///   - data: The data to push.
    ///   - options: Operation options including timeout.
    /// - Returns: The blob hash as a hex string.
    /// - Throws: `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.timeout` if the operation times out (the limit covers
    ///           hostname resolution, import and transfer),
    ///           `IrohError.pushFailed` if the push fails,
    ///           `CancellationError` if the task was cancelled.
    public func pushToNode(
        nodeId: String,
        relayUrl: String? = nil,
        directAddrs: [String] = [],
        data: Data,
        options: OperationOptions
    ) async throws -> String {
        try await pushBytes(
            nodeId: nodeId,
            relayUrl: relayUrl,
            directAddrs: directAddrs,
            data: data,
            timeoutMs: options.timeoutMs
        )
    }

    /// Shared implementation of the `Data` pushes: cancellable like the file push,
    /// so cancelling the task also stops hostname resolution and nothing is sent
    /// after a cancel. `timeoutMs` of 0 means no timeout.
    private func pushBytes(
        nodeId: String,
        relayUrl: String?,
        directAddrs: [String],
        data: Data,
        timeoutMs: UInt64
    ) async throws -> String {
        try ensureNotClosed()
        try Task.checkCancellation()

        let token = CancelTokenBox()
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
                        case IrohOperationErrorCode_TimedOut:
                            error = IrohError.timeout
                        default:
                            error = IrohError.pushFailed(message)
                        }
                        box.continuation.resume(throwing: error)
                    }
                )

                nodeId.withCString { nodeIdPtr in
                    withOptionalCString(relayUrl) { relayPtr in
                        withCStringArray(directAddrs) { addrs, count in
                            data.withUnsafeBytes { buffer in
                                let bytes = IrohBytes(
                                    data: buffer.baseAddress?.assumingMemoryBound(to: UInt8.self),
                                    len: UInt(buffer.count)
                                )
                                // The bytes are copied before this returns.
                                iroh_push_bytes_to_node(
                                    handle.pointer,
                                    nodeIdPtr,
                                    relayPtr,
                                    addrs,
                                    UInt(count),
                                    bytes,
                                    timeoutMs,
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
    ///   - directAddrs: Optional direct addresses of the remote node: `"ip:port"`
    ///     (`"[ipv6]:port"` for IPv6) or `"host:port"`. A hostname resolves to
    ///     all its A and AAAA records, through public DNS-over-HTTPS resolvers
    ///     first and the device's system resolver only if none answers; a
    ///     literal IP is used without any DNS. DNS cannot impersonate the node,
    ///     since the QUIC handshake checks `nodeId`.
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
