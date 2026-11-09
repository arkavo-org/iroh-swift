import Foundation
import Testing
@testable import IrohSwift

/// A malformed hash string must throw, not panic in Rust and abort the app.
/// iroh-blobs 0.103's `Hash::from_str` panicked on base32 input that does not
/// decode to exactly 32 bytes (e.g. "invalid"); 0.104 returns an error.
@Suite(.serialized) struct InvalidHashTests {
    static let malformed = ["invalid", "", "abc", String(repeating: "a", count: 53)]

    private func withNode(_ body: (IrohNode) async throws -> Void) async throws {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("invalid-hash-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let node = try await IrohNode(config: IrohConfig(storagePath: dir, relayEnabled: false))
        do {
            try await body(node)
        } catch {
            try? await node.close()
            throw error
        }
        try await node.close()
    }

    @Test(arguments: malformed) func createTicketRejectsMalformedHash(_ hash: String) async throws {
        try await withNode { node in
            await #expect(throws: IrohError.self) { _ = try await node.createTicket(hash: hash) }
        }
    }

    @Test(arguments: malformed) func tagBlobRejectsMalformedHash(_ hash: String) async throws {
        try await withNode { node in
            await #expect(throws: IrohError.self) { try await node.tagBlob(hash: hash, name: "t") }
        }
    }
}
