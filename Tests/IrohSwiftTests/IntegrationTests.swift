import XCTest
@testable import IrohSwift

final class IntegrationTests: XCTestCase {
    /// Test roundtrip: put data, then get it back using the ticket.
    /// This tests local storage and retrieval.
    func testLocalRoundtrip() async throws {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)

        defer {
            try? FileManager.default.removeItem(at: tempDir)
        }

        let config = IrohConfig(storagePath: tempDir, relayEnabled: false)
        let node = try await IrohNode(config: config)

        let originalData = Data("Test data for roundtrip verification".utf8)
        let ticket = try await node.put(originalData)

        print("Ticket: \(ticket)")

        // Get the data back using the same node (local retrieval)
        let retrievedData = try await node.get(ticket: ticket)

        XCTAssertEqual(retrievedData, originalData, "Retrieved data should match original")

        if let retrievedString = String(data: retrievedData, encoding: .utf8) {
            print("Retrieved: \(retrievedString)")
        }

        try await node.close()
    }
}
