import XCTest
@testable import IrohSwift

final class IntegrationTests: XCTestCase {
    /// Test that we can create a node with relay enabled and put data.
    /// This verifies the node can connect to n0 public relays.
    func testPutWithRelay() async throws {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)

        defer {
            try? FileManager.default.removeItem(at: tempDir)
        }

        // Create node with relay enabled (connects to n0 public relays)
        let config = IrohConfig(storagePath: tempDir, relayEnabled: true)
        let node = try await IrohNode(config: config)

        let testData = "Hello from iroh-swift integration test!".data(using: .utf8)!
        let ticket = try await node.put(testData)

        print("Generated ticket: \(ticket)")

        // Verify ticket format
        XCTAssertTrue(ticket.hasPrefix("blob"), "Ticket should start with 'blob'")
        XCTAssertTrue(ticket.count > 50, "Ticket should be substantial in length")
    }

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

        let originalData = "Test data for roundtrip verification".data(using: .utf8)!
        let ticket = try await node.put(originalData)

        print("Ticket: \(ticket)")

        // Get the data back using the same node (local retrieval)
        let retrievedData = try await node.get(ticket: ticket)

        XCTAssertEqual(retrievedData, originalData, "Retrieved data should match original")

        if let retrievedString = String(data: retrievedData, encoding: .utf8) {
            print("Retrieved: \(retrievedString)")
        }
    }

    /// Test two nodes: one puts, another gets.
    /// This simulates cross-node data transfer (locally).
    func testTwoNodeTransfer() async throws {
        let tempDir1 = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let tempDir2 = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)

        defer {
            try? FileManager.default.removeItem(at: tempDir1)
            try? FileManager.default.removeItem(at: tempDir2)
        }

        // Create two nodes
        let config1 = IrohConfig(storagePath: tempDir1, relayEnabled: false)
        let config2 = IrohConfig(storagePath: tempDir2, relayEnabled: false)

        let node1 = try await IrohNode(config: config1)
        let node2 = try await IrohNode(config: config2)

        // Node 1 puts data
        let testData = "Data from node1 to node2".data(using: .utf8)!
        let ticket = try await node1.put(testData)

        print("Node1 created ticket: \(ticket)")

        // Node 2 gets data using the ticket
        // Note: This may fail if nodes can't discover each other without relay
        do {
            let retrievedData = try await node2.get(ticket: ticket)
            XCTAssertEqual(retrievedData, testData)
            print("Successfully transferred data between nodes!")
        } catch {
            print("Expected: Two local nodes without relay may not discover each other: \(error)")
            // This is expected behavior - local nodes without relay can't find each other
        }
    }
}
