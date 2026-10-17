import Foundation
import IrohSwiftFFI

extension IrohNode {
    /// Download a blob by hash from one specific node into a file.
    ///
    /// Connects to `nodeId` only (no ticket, no other providers) and requests
    /// the raw blob `hash`. Every byte is verified against the BLAKE3 hash as it
    /// streams in, and is written to a hidden temporary file next to
    /// `destination`, which is renamed to `destination` only once the whole blob
    /// has been verified and flushed. The blob is not added to this node's store,
    /// so it works on client-only nodes (`servesBlobs: false`).
    ///
    /// The remote sends the blob size before any content; if it is larger than
    /// `maxSize` the fetch stops there with `IrohError.sizeLimitExceeded`. The
    /// cumulative byte count is checked again while streaming.
    ///
    /// Cancelling the calling task aborts the transfer (the connection is closed)
    /// and throws `CancellationError`. On any error or cancellation no file is
    /// left at `destination`; an existing file there is left untouched. On
    /// success an existing file at `destination` is replaced.
    ///
    /// Addressing: `directAddrs` and `relayUrl` are hints for reaching `nodeId`.
    /// A direct address is `"ip:port"` (`"[ipv6]:port"` for IPv6), used as is, or
    /// `"host:port"`: the host resolves to all its A and AAAA records, through
    /// public DNS-over-HTTPS resolvers (Cloudflare, Quad9) first and the
    /// device's system resolver only if none answers, and each becomes one
    /// address to dial. DNS cannot impersonate the node: the QUIC handshake
    /// checks `nodeId`, so a wrong answer only fails the connection. If hostnames
    /// are all that was given and none resolves, the fetch fails with
    /// `IrohError.fetchFailed`, unless this node has `IrohConfig.dhtLookup`: then
    /// it carries on with `nodeId` alone and the DHT / n0 lookup can still find
    /// the node during a DNS outage.
    ///
    /// With no `directAddrs` and no `relayUrl`, the node is looked up through the
    /// endpoint's address lookup: n0 DNS (needs relays/network enabled), plus the
    /// Mainline DHT when this node was created with `IrohConfig.dhtLookup`.
    ///
    /// - Parameters:
    ///   - hash: The blob's BLAKE3 hash, 64 hex characters.
    ///   - nodeId: The endpoint id of the node to fetch from, 64 hex characters.
    ///   - relayUrl: Optional relay URL of the remote node.
    ///   - directAddrs: Optional direct addresses of the remote node, `"ip:port"` or
    ///     `"host:port"` (see above).
    ///   - destination: File URL to write the blob to. Its directory must exist.
    ///   - maxSize: Largest blob size, in bytes, to accept.
    ///   - progress: Called with cumulative `(downloaded, total)` byte counts:
    ///     `(0, total)` once the size is accepted, then at least every 256 KiB,
    ///     and `(total, total)` last. Never decreasing. Runs on a background
    ///     thread and must return quickly.
    /// - Returns: The number of bytes written (the blob size).
    /// - Throws: `IrohError.notFound` if the remote does not have the blob,
    ///           `IrohError.sizeLimitExceeded(size)` if it is larger than `maxSize`,
    ///           `CancellationError` if the task was cancelled,
    ///           `IrohError.nodeClosed` if the node is or gets closed,
    ///           `IrohError.fetchFailed` for anything else (invalid arguments,
    ///           connection, protocol, verification or file errors).
    public func fetch(
        hash: String,
        fromNode nodeId: String,
        relayUrl: String? = nil,
        directAddrs: [String] = [],
        to destination: URL,
        maxSize: UInt64,
        progress: (@Sendable (_ downloaded: UInt64, _ total: UInt64) -> Void)? = nil
    ) async throws -> UInt64 {
        try ensureNotClosed()
        try Task.checkCancellation()
        guard destination.isFileURL else {
            throw IrohError.fetchFailed("destination must be a file URL: \(destination)")
        }

        let token = CancelTokenBox()
        let destinationPath = destination.standardizedFileURL.path
        let handle = self.handle

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<UInt64, Error>) in
                let context = FetchContext(continuation: continuation, progress: progress)
                let userdata = Unmanaged.passRetained(context).toOpaque()

                let onProgress: (@convention(c) (UnsafeMutableRawPointer?, UInt64, UInt64) -> Void)?
                if progress != nil {
                    onProgress = { userdata, downloaded, total in
                        let context = Unmanaged<FetchContext>
                            .fromOpaque(userdata!)
                            .takeUnretainedValue()
                        context.progress?(downloaded, total)
                    }
                } else {
                    onProgress = nil
                }

                let callback = IrohFetchCallback(
                    userdata: userdata,
                    on_progress: onProgress,
                    on_success: { userdata, size in
                        let context = Unmanaged<FetchContext>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        context.continuation.resume(returning: size)
                    },
                    on_failure: { userdata, code, size, errorPtr in
                        let context = Unmanaged<FetchContext>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let message = takeFFIString(errorPtr)
                        context.continuation.resume(
                            throwing: fetchError(code: code, size: size, message: message)
                        )
                    }
                )

                hash.withCString { hashPtr in
                    nodeId.withCString { nodeIdPtr in
                        destinationPath.withCString { destinationPtr in
                            withOptionalCString(relayUrl) { relayPtr in
                                withCStringArray(directAddrs) { addrs, count in
                                    iroh_fetch_from_node(
                                        handle.pointer,
                                        hashPtr,
                                        nodeIdPtr,
                                        relayPtr,
                                        addrs,
                                        UInt(count),
                                        destinationPtr,
                                        maxSize,
                                        token.pointer,
                                        callback
                                    )
                                }
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

/// Map a fetch failure from the FFI to the Swift error a caller sees.
private func fetchError(code: IrohOperationErrorCode, size: UInt64, message: String) -> Error {
    switch code {
    case IrohOperationErrorCode_NotFound:
        return IrohError.notFound
    case IrohOperationErrorCode_SizeLimitExceeded:
        return IrohError.sizeLimitExceeded(size)
    case IrohOperationErrorCode_Cancelled:
        return CancellationError()
    case IrohOperationErrorCode_NodeShutDown:
        return IrohError.nodeClosed
    default:
        return IrohError.fetchFailed(message)
    }
}

/// Context for fetch callbacks: the continuation and the optional progress handler.
private final class FetchContext: @unchecked Sendable {
    let continuation: CheckedContinuation<UInt64, Error>
    let progress: (@Sendable (UInt64, UInt64) -> Void)?

    init(
        continuation: CheckedContinuation<UInt64, Error>,
        progress: (@Sendable (UInt64, UInt64) -> Void)?
    ) {
        self.continuation = continuation
        self.progress = progress
    }
}
