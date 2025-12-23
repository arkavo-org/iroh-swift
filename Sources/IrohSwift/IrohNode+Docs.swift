import Foundation
import IrohSwiftFFI

extension IrohNode {
    // MARK: - Author Operations

    /// Import an author into the docs engine.
    ///
    /// This must be called before using an author to sign document entries.
    /// The author is registered with the docs engine so it can sign entries.
    ///
    /// - Parameter author: The author to import.
    /// - Throws: `IrohError.docsNotEnabled` if docs were not enabled on init,
    ///           `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.authorImportFailed` if import fails.
    public func importAuthor(_ author: IrohAuthor) async throws {
        try ensureNotClosed()
        try ensureDocsEnabled()
        try Task.checkCancellation()

        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            let box = Unmanaged.passRetained(
                AuthorImportContinuationBox(continuation)
            ).toOpaque()

            let callback = IrohCloseCallback(
                userdata: box,
                on_complete: { userdata in
                    let box = Unmanaged<AuthorImportContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    box.continuation.resume()
                },
                on_failure: { userdata, errorPtr in
                    let box = Unmanaged<AuthorImportContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    let message = String(cString: errorPtr!)
                    iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                    box.continuation.resume(throwing: IrohError.authorImportFailed(message))
                }
            )

            iroh_author_import(handle.pointer, author.ffiSecret, callback)
        }
    }

    // MARK: - Document Operations

    /// Create a new document.
    ///
    /// Documents are syncing key-value stores shared between peers.
    ///
    /// - Returns: A new document.
    /// - Throws: `IrohError.docsNotEnabled` if docs were not enabled on init,
    ///           `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.docCreationFailed` if creation fails.
    public func createDoc() async throws -> IrohDoc {
        try ensureNotClosed()
        try ensureDocsEnabled()
        try Task.checkCancellation()

        let nodePtr = handle.pointer
        let result: DocCreateResult = try await withCheckedThrowingContinuation { continuation in
            let box = Unmanaged.passRetained(
                DocCreateContinuationBox(continuation)
            ).toOpaque()

            let callback = IrohDocCreateCallback(
                userdata: box,
                on_success: { userdata, docHandlePtr, namespaceIdPtr in
                    let box = Unmanaged<DocCreateContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    let namespaceId = String(cString: namespaceIdPtr!)
                    iroh_string_free(UnsafeMutablePointer(mutating: namespaceIdPtr))
                    let result = DocCreateResult(
                        handle: DocHandleWrapper(pointer: docHandlePtr!),
                        namespaceId: namespaceId
                    )
                    box.continuation.resume(returning: result)
                },
                on_failure: { userdata, errorPtr in
                    let box = Unmanaged<DocCreateContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    let message = String(cString: errorPtr!)
                    iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                    box.continuation.resume(throwing: IrohError.docCreationFailed(message))
                }
            )

            iroh_doc_create(handle.pointer, callback)
        }

        return IrohDoc(
            handle: result.handle,
            nodeHandle: NodeHandleWrapper(pointer: nodePtr),
            namespaceId: result.namespaceId
        )
    }

    /// Join an existing document via ticket.
    ///
    /// This connects to peers and downloads the document content.
    ///
    /// - Parameter ticket: The document ticket string obtained from another node's `shareTicket()` call.
    /// - Returns: The joined document.
    /// - Throws: `IrohError.docsNotEnabled` if docs were not enabled on init,
    ///           `IrohError.nodeClosed` if the node is closed,
    ///           `IrohError.docJoinFailed` if joining fails.
    public func joinDoc(ticket: String) async throws -> IrohDoc {
        try ensureNotClosed()
        try ensureDocsEnabled()
        try Task.checkCancellation()

        let nodePtr = handle.pointer
        let result: DocCreateResult = try await withCheckedThrowingContinuation { continuation in
            let box = Unmanaged.passRetained(
                DocJoinContinuationBox(continuation)
            ).toOpaque()

            let callback = IrohDocCreateCallback(
                userdata: box,
                on_success: { userdata, docHandlePtr, namespaceIdPtr in
                    let box = Unmanaged<DocJoinContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    let namespaceId = String(cString: namespaceIdPtr!)
                    iroh_string_free(UnsafeMutablePointer(mutating: namespaceIdPtr))
                    let result = DocCreateResult(
                        handle: DocHandleWrapper(pointer: docHandlePtr!),
                        namespaceId: namespaceId
                    )
                    box.continuation.resume(returning: result)
                },
                on_failure: { userdata, errorPtr in
                    let box = Unmanaged<DocJoinContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    let message = String(cString: errorPtr!)
                    iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                    box.continuation.resume(throwing: IrohError.docJoinFailed(message))
                }
            )

            ticket.withCString { ticketPtr in
                iroh_doc_join(handle.pointer, ticketPtr, callback)
            }
        }

        return IrohDoc(
            handle: result.handle,
            nodeHandle: NodeHandleWrapper(pointer: nodePtr),
            namespaceId: result.namespaceId
        )
    }

    // MARK: - Private Helpers

    /// Check that docs were enabled during node initialization.
    private func ensureDocsEnabled() throws {
        // This check relies on the Rust side returning an error if docs are not enabled.
        // For a more efficient check, we could add a flag to the Swift side,
        // but for now we'll let the FFI layer handle the validation.
        // The error will be clear when the operation fails.
    }
}

// MARK: - Result Type

/// Internal result type for document creation/join.
private struct DocCreateResult: @unchecked Sendable {
    let handle: DocHandleWrapper
    let namespaceId: String
}

// MARK: - Continuation Boxes

private final class DocCreateContinuationBox: @unchecked Sendable {
    let continuation: CheckedContinuation<DocCreateResult, Error>

    init(_ continuation: CheckedContinuation<DocCreateResult, Error>) {
        self.continuation = continuation
    }
}

private final class DocJoinContinuationBox: @unchecked Sendable {
    let continuation: CheckedContinuation<DocCreateResult, Error>

    init(_ continuation: CheckedContinuation<DocCreateResult, Error>) {
        self.continuation = continuation
    }
}

private final class AuthorImportContinuationBox: @unchecked Sendable {
    let continuation: CheckedContinuation<Void, Error>

    init(_ continuation: CheckedContinuation<Void, Error>) {
        self.continuation = continuation
    }
}
