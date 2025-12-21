import XCTest
@testable import IrohSwift

final class IrohNodeTests: XCTestCase {
    /// Test that we can create a node with default config.
    func testNodeCreation() async throws {
        // Use a temp directory to avoid polluting the system
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)

        defer {
            try? FileManager.default.removeItem(at: tempDir)
        }

        let config = IrohConfig(storagePath: tempDir, relayEnabled: false)
        let node = try await IrohNode(config: config)

        // If we get here without throwing, node creation succeeded
        _ = node
    }

    /// Test putting data returns a valid ticket.
    func testPut() async throws {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)

        defer {
            try? FileManager.default.removeItem(at: tempDir)
        }

        let config = IrohConfig(storagePath: tempDir, relayEnabled: false)
        let node = try await IrohNode(config: config)

        let data = Data("Hello, Iroh!".utf8)
        let ticket = try await node.put(data)

        // Tickets should start with "blob" (BlobTicket format)
        XCTAssertTrue(ticket.hasPrefix("blob"), "Ticket should start with 'blob', got: \(ticket)")
        XCTAssertFalse(ticket.isEmpty, "Ticket should not be empty")
    }

    /// Test putting empty data.
    func testPutEmptyData() async throws {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)

        defer {
            try? FileManager.default.removeItem(at: tempDir)
        }

        let config = IrohConfig(storagePath: tempDir, relayEnabled: false)
        let node = try await IrohNode(config: config)

        let data = Data()
        let ticket = try await node.put(data)

        XCTAssertTrue(ticket.hasPrefix("blob"), "Ticket should start with 'blob'")
    }

    /// Test that IrohConfig uses Application Support by default.
    func testDefaultStoragePath() {
        let config = IrohConfig()

        // Should be in Application Support
        XCTAssertTrue(
            config.storagePath.path.contains("Application Support"),
            "Default path should be in Application Support, got: \(config.storagePath.path)"
        )
        XCTAssertTrue(
            config.storagePath.path.hasSuffix("iroh"),
            "Default path should end with 'iroh'"
        )
    }

    /// Test that relay is enabled by default.
    func testDefaultRelayEnabled() {
        let config = IrohConfig()
        XCTAssertTrue(config.relayEnabled, "Relay should be enabled by default")
    }

    /// Test config with custom values.
    func testCustomConfig() {
        let customPath = FileManager.default.temporaryDirectory
            .appendingPathComponent("custom-iroh", isDirectory: true)

        let config = IrohConfig(storagePath: customPath, relayEnabled: false)

        XCTAssertEqual(config.storagePath, customPath)
        XCTAssertFalse(config.relayEnabled)
    }
}
