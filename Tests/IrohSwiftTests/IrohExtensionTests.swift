import XCTest
@testable import IrohSwift

final class IrohExtensionTests: XCTestCase {

    private func createTestNode() async throws -> IrohNode {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let config = IrohConfig(storagePath: tempDir, relayEnabled: false)
        return try await IrohNode(config: config)
    }

    // MARK: - String Tests

    func testPutString() async throws {
        let node = try await createTestNode()
        let ticket = try await node.put("Hello, Iroh!", encoding: .utf8)
        XCTAssertTrue(ticket.hasPrefix("blob"), "Ticket should start with 'blob'")
    }

    func testGetString() async throws {
        let node = try await createTestNode()
        let original = "Hello, Iroh!"
        let ticket = try await node.put(original, encoding: .utf8)
        let retrieved = try await node.getString(ticket: ticket, encoding: .utf8)
        XCTAssertEqual(retrieved, original)
    }

    func testStringEncodingFailure() async throws {
        let node = try await createTestNode()
        // Emoji string can't be encoded in ASCII
        let emoji = "Hello 👋"

        do {
            _ = try await node.put(emoji, encoding: .ascii)
            XCTFail("Should have thrown stringEncodingFailed")
        } catch IrohError.stringEncodingFailed {
            // Expected
        }
    }

    // MARK: - Codable Tests

    struct TestModel: Codable, Equatable {
        let name: String
        let value: Int
    }

    func testPutCodable() async throws {
        let node = try await createTestNode()
        let model = TestModel(name: "test", value: 42)
        let ticket = try await node.put(model)
        XCTAssertTrue(ticket.hasPrefix("blob"), "Ticket should start with 'blob'")
    }

    func testGetCodable() async throws {
        let node = try await createTestNode()
        let original = TestModel(name: "test", value: 42)
        let ticket = try await node.put(original)
        let retrieved = try await node.get(ticket: ticket, as: TestModel.self)
        XCTAssertEqual(retrieved, original)
    }

    func testDecodingFailure() async throws {
        let node = try await createTestNode()
        // Store plain string, try to decode as struct
        let ticket = try await node.put(Data("not json".utf8))

        do {
            _ = try await node.get(ticket: ticket, as: TestModel.self)
            XCTFail("Should have thrown decodingFailed")
        } catch IrohError.decodingFailed {
            // Expected
        }
    }

    // MARK: - Ticket Validation Tests

    func testValidTicketFormat() {
        XCTAssertTrue(IrohNode.isValidTicket("blobABCDEF123456"))
        XCTAssertFalse(IrohNode.isValidTicket(""))
        XCTAssertFalse(IrohNode.isValidTicket("invalid"))
        XCTAssertFalse(IrohNode.isValidTicket("blob"))  // Too short
        XCTAssertFalse(IrohNode.isValidTicket("Blob123456"))  // Wrong case
    }

    func testValidateTicketWithValidTicket() async throws {
        let node = try await createTestNode()
        let data = Data("test data".utf8)
        let ticket = try await node.put(data)

        let info = await validateTicket(ticket)
        XCTAssertTrue(info.isValid, "Ticket should be valid")
        XCTAssertNotNil(info.hash, "Should have hash")
        XCTAssertNotNil(info.nodeId, "Should have node ID")
    }

    func testValidateTicketWithInvalidTicket() async {
        let info = await validateTicket("not-a-valid-ticket")
        XCTAssertFalse(info.isValid, "Invalid ticket should not be valid")
        XCTAssertNil(info.hash)
        XCTAssertNil(info.nodeId)
    }

    // MARK: - Retry Tests

    func testPutWithRetrySuccess() async throws {
        let node = try await createTestNode()
        let data = Data("test data".utf8)
        let ticket = try await node.putWithRetry(data, maxAttempts: 3)
        XCTAssertTrue(ticket.hasPrefix("blob"), "Ticket should start with 'blob'")
    }

    func testGetWithRetrySuccess() async throws {
        let node = try await createTestNode()
        let data = Data("test data".utf8)
        let ticket = try await node.put(data)
        let retrieved = try await node.getWithRetry(ticket: ticket, maxAttempts: 3)
        XCTAssertEqual(retrieved, data)
    }

    // MARK: - Logging Tests

    func testPutWithLogging() async throws {
        let node = try await createTestNode()
        let data = Data("test data".utf8)
        let ticket = try await node.putWithLogging(data)
        XCTAssertTrue(ticket.hasPrefix("blob"), "Ticket should start with 'blob'")
    }

    func testGetWithLogging() async throws {
        let node = try await createTestNode()
        let data = Data("test data".utf8)
        let ticket = try await node.put(data)
        let retrieved = try await node.getWithLogging(ticket: ticket)
        XCTAssertEqual(retrieved, data)
    }

    // MARK: - Node Info Tests

    func testNodeInfo() async throws {
        let node = try await createTestNode()
        let info = try await node.info()
        XCTAssertFalse(info.nodeId.isEmpty, "Node ID should not be empty")
        // Without relay, relay URL should be nil
        XCTAssertNil(info.relayUrl, "Without relay, relay URL should be nil")
    }

    // MARK: - Progress Tests

    func testGetWithProgress() async throws {
        let node = try await createTestNode()
        let testData = Data(repeating: 0x42, count: 1024)
        let ticket = try await node.put(testData)

        let retrieved = try await node.get(ticket: ticket) { _ in
            // Progress callback - may or may not be called for local gets
        }

        XCTAssertEqual(retrieved, testData)
    }

    // MARK: - Cancellation Tests

    func testCancellationCheckInPut() async throws {
        let node = try await createTestNode()

        let task = Task {
            try await node.put(Data("test".utf8))
        }

        task.cancel()

        do {
            _ = try await task.value
            // May or may not throw depending on timing
        } catch is CancellationError {
            // Expected if cancelled before operation started
        } catch {
            // Other errors are also acceptable
        }
    }
}
