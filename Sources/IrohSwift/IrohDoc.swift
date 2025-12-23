import Foundation
import IrohSwiftFFI

/// Thread-safe Iroh document for syncing key-value data.
///
/// IrohDoc is implemented as a Swift actor to ensure thread-safe access.
/// Documents are syncing key-value stores shared between peers.
///
/// Example usage:
/// ```swift
/// let node = try await IrohNode(config: IrohConfig(docsEnabled: true))
/// let author = try await IrohAuthor.getOrCreate()
///
/// // Create a new document
/// let doc = try await node.createDoc()
///
/// // Write data
/// try await doc.set(author: author, key: "greeting", value: "Hello!".data(using: .utf8)!)
///
/// // Read data
/// if let entry = try await doc.get(key: "greeting") {
///     let data = try await entry.content(from: doc)
///     print(String(data: data, encoding: .utf8)!)
/// }
///
/// // Share with others
/// let ticket = try await doc.shareTicket(mode: .write)
/// ```
public actor IrohDoc {
    let handle: DocHandleWrapper
    let nodeHandle: NodeHandleWrapper

    /// The namespace ID for this document.
    public let namespaceId: String

    private var isClosed = false

    /// Create a document from FFI handles.
    init(handle: DocHandleWrapper,
         nodeHandle: NodeHandleWrapper,
         namespaceId: String) {
        self.handle = handle
        self.nodeHandle = nodeHandle
        self.namespaceId = namespaceId
    }

    deinit {
        if !isClosed {
            iroh_doc_close(handle.pointer)
        }
    }

    // MARK: - Lifecycle

    /// Close the document and release resources.
    ///
    /// After calling close(), the document cannot be used for any operations.
    public func close() {
        guard !isClosed else { return }
        isClosed = true
        iroh_doc_close(handle.pointer)
    }

    /// Check if the document has been closed.
    func ensureNotClosed() throws {
        if isClosed {
            throw IrohError.docClosed
        }
    }

    // MARK: - CRUD Operations

    /// Set a key-value pair in the document.
    ///
    /// - Parameters:
    ///   - author: The author signing this entry.
    ///   - key: The key as a string (UTF-8 encoded).
    ///   - value: The value data.
    /// - Returns: The content hash of the stored value.
    /// - Throws: `IrohError.docClosed` if the document is closed,
    ///           `IrohError.docSetFailed` if the operation fails.
    public func set(author: IrohAuthor, key: String, value: Data) async throws -> String {
        try ensureNotClosed()
        guard let keyData = key.data(using: .utf8) else {
            throw IrohError.stringEncodingFailed(.utf8)
        }
        return try await set(author: author, key: keyData, value: value)
    }

    /// Set a key-value pair in the document using raw key bytes.
    ///
    /// - Parameters:
    ///   - author: The author signing this entry.
    ///   - key: The key bytes.
    ///   - value: The value data.
    /// - Returns: The content hash of the stored value.
    /// - Throws: `IrohError.docClosed` if the document is closed,
    ///           `IrohError.docSetFailed` if the operation fails.
    public func set(author: IrohAuthor, key: Data, value: Data) async throws -> String {
        try ensureNotClosed()
        try Task.checkCancellation()

        return try await withCheckedThrowingContinuation { continuation in
            key.withUnsafeBytes { keyBuffer in
                value.withUnsafeBytes { valueBuffer in
                    let keyBytes = IrohBytes(
                        data: keyBuffer.baseAddress?.assumingMemoryBound(to: UInt8.self),
                        len: UInt(keyBuffer.count)
                    )
                    let valueBytes = IrohBytes(
                        data: valueBuffer.baseAddress?.assumingMemoryBound(to: UInt8.self),
                        len: UInt(valueBuffer.count)
                    )

                    let box = Unmanaged.passRetained(
                        StringContinuationBox(continuation)
                    ).toOpaque()

                    let callback = IrohDocSetCallback(
                        userdata: box,
                        on_success: { userdata, hashPtr in
                            let box = Unmanaged<StringContinuationBox>
                                .fromOpaque(userdata!)
                                .takeRetainedValue()
                            let hash = String(cString: hashPtr!)
                            iroh_string_free(UnsafeMutablePointer(mutating: hashPtr))
                            box.continuation.resume(returning: hash)
                        },
                        on_failure: { userdata, errorPtr in
                            let box = Unmanaged<StringContinuationBox>
                                .fromOpaque(userdata!)
                                .takeRetainedValue()
                            let message = String(cString: errorPtr!)
                            iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                            box.continuation.resume(throwing: IrohError.docSetFailed(message))
                        }
                    )

                    iroh_doc_set(handle.pointer, author.ffiSecret, keyBytes, valueBytes, callback)
                }
            }
        }
    }

    /// Get the latest entry for a key.
    ///
    /// - Parameter key: The key as a string (UTF-8 encoded).
    /// - Returns: The entry if found, nil if not found.
    /// - Throws: `IrohError.docClosed` if the document is closed,
    ///           `IrohError.docGetFailed` if the operation fails.
    public func get(key: String) async throws -> DocEntry? {
        try ensureNotClosed()
        guard let keyData = key.data(using: .utf8) else {
            throw IrohError.stringEncodingFailed(.utf8)
        }
        return try await get(key: keyData)
    }

    /// Get the latest entry for a key using raw key bytes.
    ///
    /// - Parameter key: The key bytes.
    /// - Returns: The entry if found, nil if not found.
    /// - Throws: `IrohError.docClosed` if the document is closed,
    ///           `IrohError.docGetFailed` if the operation fails.
    public func get(key: Data) async throws -> DocEntry? {
        try ensureNotClosed()
        try Task.checkCancellation()

        return try await withCheckedThrowingContinuation { continuation in
            key.withUnsafeBytes { keyBuffer in
                let keyBytes = IrohBytes(
                    data: keyBuffer.baseAddress?.assumingMemoryBound(to: UInt8.self),
                    len: UInt(keyBuffer.count)
                )

                let box = Unmanaged.passRetained(
                    DocEntryContinuationBox(continuation)
                ).toOpaque()

                let callback = IrohDocGetCallback(
                    userdata: box,
                    on_success: { userdata, entryPtr in
                        let box = Unmanaged<DocEntryContinuationBox>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()

                        if let entryPtr = entryPtr {
                            let entry = DocEntry(from: entryPtr.pointee)
                            iroh_doc_entry_free(UnsafeMutablePointer(mutating: entryPtr))
                            box.continuation.resume(returning: entry)
                        } else {
                            box.continuation.resume(returning: nil)
                        }
                    },
                    on_failure: { userdata, errorPtr in
                        let box = Unmanaged<DocEntryContinuationBox>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let message = String(cString: errorPtr!)
                        iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                        box.continuation.resume(throwing: IrohError.docGetFailed(message))
                    }
                )

                iroh_doc_get(handle.pointer, keyBytes, callback)
            }
        }
    }

    /// Get entries by key prefix.
    ///
    /// - Parameter prefix: The key prefix as a string (UTF-8 encoded).
    /// - Returns: An async stream of entries matching the prefix.
    /// - Throws: `IrohError.docClosed` if the document is closed.
    public func getMany(prefix: String) async throws -> AsyncThrowingStream<DocEntry, Error> {
        try ensureNotClosed()
        guard let prefixData = prefix.data(using: .utf8) else {
            throw IrohError.stringEncodingFailed(.utf8)
        }
        return try await getMany(prefix: prefixData)
    }

    /// Get entries by key prefix using raw bytes.
    ///
    /// - Parameter prefix: The key prefix bytes.
    /// - Returns: An async stream of entries matching the prefix.
    /// - Throws: `IrohError.docClosed` if the document is closed.
    public func getMany(prefix: Data) async throws -> AsyncThrowingStream<DocEntry, Error> {
        try ensureNotClosed()

        return AsyncThrowingStream(bufferingPolicy: .bufferingNewest(100)) { continuation in
            let context = GetManyContext(continuation: continuation)
            let contextPtr = Unmanaged.passRetained(context).toOpaque()

            prefix.withUnsafeBytes { prefixBuffer in
                let prefixBytes = IrohBytes(
                    data: prefixBuffer.baseAddress?.assumingMemoryBound(to: UInt8.self),
                    len: UInt(prefixBuffer.count)
                )

                let callback = IrohDocGetManyCallback(
                    userdata: contextPtr,
                    on_entry: { userdata, entryPtr in
                        let ctx = Unmanaged<GetManyContext>
                            .fromOpaque(userdata!)
                            .takeUnretainedValue()  // Don't consume - more entries coming
                        let entry = DocEntry(from: entryPtr!.pointee)
                        iroh_doc_entry_free(UnsafeMutablePointer(mutating: entryPtr))
                        ctx.continuation.yield(entry)
                    },
                    on_complete: { userdata in
                        let ctx = Unmanaged<GetManyContext>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()  // Consume on terminal
                        ctx.continuation.finish()
                    },
                    on_failure: { userdata, errorPtr in
                        let ctx = Unmanaged<GetManyContext>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()  // Consume on terminal
                        let message = String(cString: errorPtr!)
                        iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                        ctx.continuation.finish(throwing: IrohError.docGetFailed(message))
                    }
                )

                iroh_doc_get_many(handle.pointer, prefixBytes, callback)
            }
        }
    }

    /// Delete an entry (creates a tombstone).
    ///
    /// - Parameters:
    ///   - author: The author signing the deletion.
    ///   - key: The key as a string (UTF-8 encoded).
    /// - Returns: The number of entries deleted.
    /// - Throws: `IrohError.docClosed` if the document is closed,
    ///           `IrohError.docDeleteFailed` if the operation fails.
    public func delete(author: IrohAuthor, key: String) async throws -> UInt64 {
        try ensureNotClosed()
        guard let keyData = key.data(using: .utf8) else {
            throw IrohError.stringEncodingFailed(.utf8)
        }
        return try await delete(author: author, key: keyData)
    }

    /// Delete an entry using raw key bytes.
    ///
    /// - Parameters:
    ///   - author: The author signing the deletion.
    ///   - key: The key bytes.
    /// - Returns: The number of entries deleted.
    /// - Throws: `IrohError.docClosed` if the document is closed,
    ///           `IrohError.docDeleteFailed` if the operation fails.
    public func delete(author: IrohAuthor, key: Data) async throws -> UInt64 {
        try ensureNotClosed()
        try Task.checkCancellation()

        return try await withCheckedThrowingContinuation { continuation in
            key.withUnsafeBytes { keyBuffer in
                let keyBytes = IrohBytes(
                    data: keyBuffer.baseAddress?.assumingMemoryBound(to: UInt8.self),
                    len: UInt(keyBuffer.count)
                )

                let box = Unmanaged.passRetained(
                    DeleteContinuationBox(continuation)
                ).toOpaque()

                let callback = IrohDocDelCallback(
                    userdata: box,
                    on_success: { userdata, deletedCount in
                        let box = Unmanaged<DeleteContinuationBox>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        box.continuation.resume(returning: deletedCount)
                    },
                    on_failure: { userdata, errorPtr in
                        let box = Unmanaged<DeleteContinuationBox>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let message = String(cString: errorPtr!)
                        iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                        box.continuation.resume(throwing: IrohError.docDeleteFailed(message))
                    }
                )

                iroh_doc_del(handle.pointer, author.ffiSecret, keyBytes, callback)
            }
        }
    }

    // MARK: - Content

    /// Read content bytes by hash.
    ///
    /// Entries only contain a hash of the content. Use this method to
    /// retrieve the actual bytes.
    ///
    /// - Parameter hash: The content hash as a hex string.
    /// - Returns: The content data.
    /// - Throws: `IrohError.docClosed` if the document is closed,
    ///           `IrohError.contentReadFailed` if reading fails.
    public func readContent(hash: String) async throws -> Data {
        try ensureNotClosed()
        try Task.checkCancellation()

        return try await withCheckedThrowingContinuation { continuation in
            hash.withCString { hashPtr in
                let box = Unmanaged.passRetained(
                    DataContinuationBox(continuation)
                ).toOpaque()

                let callback = IrohGetCallback(
                    userdata: box,
                    on_success: { userdata, ownedBytes in
                        let box = Unmanaged<DataContinuationBox>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let data = Data(bytes: ownedBytes.data, count: Int(ownedBytes.len))
                        iroh_bytes_free(ownedBytes)
                        box.continuation.resume(returning: data)
                    },
                    on_failure: { userdata, errorPtr in
                        let box = Unmanaged<DataContinuationBox>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let message = String(cString: errorPtr!)
                        iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                        box.continuation.resume(throwing: IrohError.contentReadFailed(message))
                    }
                )

                iroh_doc_read_content(nodeHandle.pointer, hashPtr, callback)
            }
        }
    }

    // MARK: - Sharing

    /// Get a share ticket for this document.
    ///
    /// - Parameter mode: The access mode (read or write). Default is read.
    /// - Returns: A ticket string that can be used to join this document.
    /// - Throws: `IrohError.docClosed` if the document is closed,
    ///           `IrohError.docShareFailed` if sharing fails.
    public func shareTicket(mode: DocShareMode = .read) async throws -> String {
        try ensureNotClosed()
        try Task.checkCancellation()

        return try await withCheckedThrowingContinuation { continuation in
            let box = Unmanaged.passRetained(
                StringContinuationBox(continuation)
            ).toOpaque()

            let callback = IrohCallback(
                userdata: box,
                on_success: { userdata, ticketPtr in
                    let box = Unmanaged<StringContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    let ticket = String(cString: ticketPtr!)
                    iroh_string_free(UnsafeMutablePointer(mutating: ticketPtr))
                    box.continuation.resume(returning: ticket)
                },
                on_failure: { userdata, errorPtr in
                    let box = Unmanaged<StringContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    let message = String(cString: errorPtr!)
                    iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                    box.continuation.resume(throwing: IrohError.docShareFailed(message))
                }
            )

            iroh_doc_share(handle.pointer, mode.ffiMode, callback)
        }
    }
}

// MARK: - Handle Wrapper

/// Sendable wrapper for the document handle pointer.
struct DocHandleWrapper: @unchecked Sendable {
    let pointer: UnsafeMutablePointer<IrohDocHandle>
}

// MARK: - Continuation Boxes

private final class StringContinuationBox: @unchecked Sendable {
    let continuation: CheckedContinuation<String, Error>

    init(_ continuation: CheckedContinuation<String, Error>) {
        self.continuation = continuation
    }
}

private final class DataContinuationBox: @unchecked Sendable {
    let continuation: CheckedContinuation<Data, Error>

    init(_ continuation: CheckedContinuation<Data, Error>) {
        self.continuation = continuation
    }
}

private final class DocEntryContinuationBox: @unchecked Sendable {
    let continuation: CheckedContinuation<DocEntry?, Error>

    init(_ continuation: CheckedContinuation<DocEntry?, Error>) {
        self.continuation = continuation
    }
}

private final class DeleteContinuationBox: @unchecked Sendable {
    let continuation: CheckedContinuation<UInt64, Error>

    init(_ continuation: CheckedContinuation<UInt64, Error>) {
        self.continuation = continuation
    }
}

private final class GetManyContext: @unchecked Sendable {
    let continuation: AsyncThrowingStream<DocEntry, Error>.Continuation

    init(continuation: AsyncThrowingStream<DocEntry, Error>.Continuation) {
        self.continuation = continuation
    }
}
