import Foundation

extension IrohNode {
    /// Store an encodable value and return a shareable ticket.
    ///
    /// - Parameters:
    ///   - value: The value to encode and store.
    ///   - encoder: The JSON encoder to use. Defaults to a standard encoder.
    /// - Returns: A ticket string for retrieving the data.
    /// - Throws: `IrohError.encodingFailed` if encoding fails,
    ///           or `IrohError.putFailed` if storage fails.
    public func put<T: Encodable>(
        _ value: T,
        encoder: JSONEncoder = JSONEncoder()
    ) async throws -> String {
        try Task.checkCancellation()

        let data: Data
        do {
            data = try encoder.encode(value)
        } catch {
            throw IrohError.encodingFailed(error.localizedDescription)
        }

        return try await put(data)
    }

    /// Retrieve and decode a value from a ticket.
    ///
    /// - Parameters:
    ///   - ticket: The ticket string from a `put` operation.
    ///   - type: The type to decode into.
    ///   - decoder: The JSON decoder to use. Defaults to a standard decoder.
    /// - Returns: The decoded value.
    /// - Throws: `IrohError.decodingFailed` if decoding fails,
    ///           or `IrohError.getFailed` if retrieval fails.
    public func get<T: Decodable>(
        ticket: String,
        as type: T.Type,
        decoder: JSONDecoder = JSONDecoder()
    ) async throws -> T {
        try Task.checkCancellation()

        let data = try await get(ticket: ticket)

        do {
            return try decoder.decode(type, from: data)
        } catch {
            throw IrohError.decodingFailed(error.localizedDescription)
        }
    }
}
