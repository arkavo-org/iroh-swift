import Foundation

extension IrohNode {
    /// Store a string and return a shareable ticket.
    ///
    /// - Parameters:
    ///   - string: The string to store.
    ///   - encoding: The string encoding to use. Defaults to UTF-8.
    /// - Returns: A ticket string for retrieving the data.
    /// - Throws: `IrohError.stringEncodingFailed` if encoding fails,
    ///           or `IrohError.putFailed` if storage fails.
    public func put(_ string: String, encoding: String.Encoding = .utf8) async throws -> String {
        try Task.checkCancellation()

        guard let data = string.data(using: encoding) else {
            throw IrohError.stringEncodingFailed(encoding)
        }

        return try await put(data)
    }

    /// Retrieve a string from a ticket.
    ///
    /// - Parameters:
    ///   - ticket: The ticket string from a `put` operation.
    ///   - encoding: The string encoding to use. Defaults to UTF-8.
    /// - Returns: The decoded string.
    /// - Throws: `IrohError.stringDecodingFailed` if decoding fails,
    ///           or `IrohError.getFailed` if retrieval fails.
    public func getString(ticket: String, encoding: String.Encoding = .utf8) async throws -> String {
        try Task.checkCancellation()

        let data = try await get(ticket: ticket)

        guard let string = String(data: data, encoding: encoding) else {
            throw IrohError.stringDecodingFailed(encoding)
        }

        return string
    }
}
