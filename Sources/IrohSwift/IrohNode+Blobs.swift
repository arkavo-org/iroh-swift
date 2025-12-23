import Foundation
import IrohSwiftFFI

extension IrohNode {
    // MARK: - Blob Tag Operations

    /// Tag (pin) a blob to prevent garbage collection.
    ///
    /// Tagged blobs are protected from GC until the tag is removed.
    /// Use this after downloading content you want to keep permanently.
    ///
    /// Example usage:
    /// ```swift
    /// // After downloading content
    /// let _ = try await node.get(ticket)
    ///
    /// // Pin it to prevent GC
    /// let ticketInfo = try await node.validateTicket(ticket)
    /// try await node.tagBlob(hash: ticketInfo.hash!, name: "pins/my-content")
    /// ```
    ///
    /// - Parameters:
    ///   - hash: The blob hash (hex string from ticket or entry).
    ///   - name: Tag name (e.g., "pins/my-content").
    ///   - format: Blob format (default: .raw).
    /// - Throws: `IrohError.blobTagFailed` if tagging fails.
    public func tagBlob(hash: String, name: String, format: BlobFormat = .raw) async throws {
        try ensureNotClosed()
        try Task.checkCancellation()

        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            let box = Unmanaged.passRetained(
                BlobTagContinuationBox(continuation)
            ).toOpaque()

            let callback = IrohCloseCallback(
                userdata: box,
                on_complete: { userdata in
                    let box = Unmanaged<BlobTagContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    box.continuation.resume()
                },
                on_failure: { userdata, errorPtr in
                    let box = Unmanaged<BlobTagContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    let message = String(cString: errorPtr!)
                    iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                    box.continuation.resume(throwing: IrohError.blobTagFailed(message))
                }
            )

            let ffiFormat: IrohBlobFormat = format == .raw ? Raw : HashSeq

            name.withCString { namePtr in
                hash.withCString { hashPtr in
                    iroh_blob_tag_set(handle.pointer, namePtr, hashPtr, ffiFormat, callback)
                }
            }
        }
    }

    /// Create a shareable ticket for an existing local blob.
    ///
    /// The ticket points to this node as the provider.
    /// Use this to "mint" a bootstrap ticket after downloading content,
    /// allowing others to fetch from this node instead of the original source.
    ///
    /// Example usage:
    /// ```swift
    /// // After downloading and pinning content
    /// let bootstrapTicket = try await node.createTicket(hash: contentHash)
    ///
    /// // Share the bootstrap ticket (points to this node)
    /// print("Fetch from me: \(bootstrapTicket)")
    /// ```
    ///
    /// - Parameters:
    ///   - hash: The blob hash (hex string).
    ///   - format: Blob format (default: .raw).
    /// - Returns: A shareable ticket string.
    /// - Throws: `IrohError.ticketCreationFailed` if ticket creation fails.
    public func createTicket(hash: String, format: BlobFormat = .raw) async throws -> String {
        try ensureNotClosed()
        try Task.checkCancellation()

        return try await withCheckedThrowingContinuation { continuation in
            let box = Unmanaged.passRetained(
                TicketCreateContinuationBox(continuation)
            ).toOpaque()

            let callback = IrohCallback(
                userdata: box,
                on_success: { userdata, ticketPtr in
                    let box = Unmanaged<TicketCreateContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    let ticket = String(cString: ticketPtr!)
                    iroh_string_free(UnsafeMutablePointer(mutating: ticketPtr))
                    box.continuation.resume(returning: ticket)
                },
                on_failure: { userdata, errorPtr in
                    let box = Unmanaged<TicketCreateContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    let message = String(cString: errorPtr!)
                    iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                    box.continuation.resume(throwing: IrohError.ticketCreationFailed(message))
                }
            )

            let ffiFormat: IrohBlobFormat = format == .raw ? Raw : HashSeq

            hash.withCString { hashPtr in
                iroh_blob_ticket_create(handle.pointer, hashPtr, ffiFormat, callback)
            }
        }
    }
}

// MARK: - Continuation Boxes

private final class BlobTagContinuationBox: @unchecked Sendable {
    let continuation: CheckedContinuation<Void, Error>

    init(_ continuation: CheckedContinuation<Void, Error>) {
        self.continuation = continuation
    }
}

private final class TicketCreateContinuationBox: @unchecked Sendable {
    let continuation: CheckedContinuation<String, Error>

    init(_ continuation: CheckedContinuation<String, Error>) {
        self.continuation = continuation
    }
}
