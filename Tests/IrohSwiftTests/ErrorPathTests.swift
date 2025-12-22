import Foundation
import Testing
@testable import IrohSwift

/// Tests for error handling and edge cases.
struct ErrorPathTests {
    // MARK: - Configuration Validation Tests

    @Test("Invalid storage path throws invalidConfiguration")
    func testInvalidStoragePath() async throws {
        // Use a path that definitely won't be writable
        let invalidPath = URL(fileURLWithPath: "/nonexistent/readonly/path/iroh")
        let config = IrohConfig(storagePath: invalidPath)

        do {
            try config.validate()
            #expect(Bool(false), "Should have thrown invalidConfiguration")
        } catch let error as IrohError {
            switch error {
            case .invalidConfiguration(let msg):
                #expect(msg.contains("Storage path") || msg.contains("Cannot create"))
            default:
                #expect(Bool(false), "Expected invalidConfiguration, got \(error)")
            }
        }
    }

    @Test("Invalid relay URL format throws invalidConfiguration")
    func testInvalidRelayUrl() async throws {
        // Use a temp directory to ensure storage validation passes
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        var config = IrohConfig(storagePath: tempDir)
        config.customRelayUrl = "not-a-valid-url"

        do {
            try config.validate()
            #expect(Bool(false), "Should have thrown invalidConfiguration")
        } catch let error as IrohError {
            switch error {
            case .invalidConfiguration(let msg):
                #expect(msg.contains("Invalid relay URL"))
            default:
                #expect(Bool(false), "Expected invalidConfiguration, got \(error)")
            }
        }
    }

    @Test("Relay URL without https throws invalidConfiguration")
    func testRelayUrlWithoutHttps() async throws {
        // Use a temp directory to ensure storage validation passes
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        var config = IrohConfig(storagePath: tempDir)
        config.customRelayUrl = "ftp://relay.example.com"

        do {
            try config.validate()
            #expect(Bool(false), "Should have thrown invalidConfiguration")
        } catch let error as IrohError {
            switch error {
            case .invalidConfiguration(let msg):
                #expect(msg.contains("Invalid relay URL"))
            default:
                #expect(Bool(false), "Expected invalidConfiguration, got \(error)")
            }
        }
    }

    @Test("Valid config with custom relay passes validation")
    func testValidConfigWithCustomRelay() async throws {
        // Use a temp directory for testing
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        var config = IrohConfig(storagePath: tempDir)
        config.customRelayUrl = "https://relay.example.com"

        // This should not throw
        try config.validate()
    }

    // MARK: - Ticket Validation Tests

    @Test("Invalid ticket format returns isValid=false")
    func testInvalidTicketFormat() async {
        let info = await validateTicket("not-a-valid-ticket")
        #expect(!info.isValid)
        #expect(info.hash == nil)
        #expect(info.nodeId == nil)
    }

    @Test("Empty ticket returns isValid=false")
    func testEmptyTicket() async {
        let info = await validateTicket("")
        #expect(!info.isValid)
    }

    @Test("Truncated ticket returns isValid=false")
    func testTruncatedTicket() async {
        // A ticket that starts correctly but is truncated
        let info = await validateTicket("blobafk2bi")
        #expect(!info.isValid)
    }

    // MARK: - Node Close Tests

    @Test("Double close is safe")
    func testDoubleClose() async throws {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString)

        let config = IrohConfig(storagePath: tempDir, relayEnabled: false)
        let node = try await IrohNode(config: config)

        // First close should succeed
        try await node.close()

        // Second close should be safe (no-op)
        try await node.close()

        // Cleanup
        try? FileManager.default.removeItem(at: tempDir)
    }

    @Test("Operation after close throws nodeClosed")
    func testOperationAfterClose() async throws {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString)

        let config = IrohConfig(storagePath: tempDir, relayEnabled: false)
        let node = try await IrohNode(config: config)

        try await node.close()

        do {
            _ = try await node.put(Data("test".utf8))
            #expect(Bool(false), "Should have thrown nodeClosed")
        } catch let error as IrohError {
            switch error {
            case .nodeClosed:
                break // Expected
            default:
                #expect(Bool(false), "Expected nodeClosed, got \(error)")
            }
        }

        // Cleanup
        try? FileManager.default.removeItem(at: tempDir)
    }

    @Test("Get after close throws nodeClosed")
    func testGetAfterClose() async throws {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString)

        let config = IrohConfig(storagePath: tempDir, relayEnabled: false)
        let node = try await IrohNode(config: config)

        try await node.close()

        do {
            _ = try await node.get(ticket: "blobafk...")
            #expect(Bool(false), "Should have thrown nodeClosed")
        } catch let error as IrohError {
            switch error {
            case .nodeClosed:
                break // Expected
            default:
                #expect(Bool(false), "Expected nodeClosed, got \(error)")
            }
        }

        // Cleanup
        try? FileManager.default.removeItem(at: tempDir)
    }

    // MARK: - Error Descriptions

    @Test("All error types have descriptions")
    func testErrorDescriptions() {
        let errors: [IrohError] = [
            .nodeCreationFailed("test"),
            .putFailed("test"),
            .getFailed("test"),
            .invalidTicket("test"),
            .stringEncodingFailed(.utf8),
            .stringDecodingFailed(.utf8),
            .encodingFailed("test"),
            .decodingFailed("test"),
            .maxRetriesExceeded(attempts: 3, lastError: IrohError.timeout),
            .invalidConfiguration("test"),
            .timeout,
            .nodeClosed,
            .closeFailed("test")
        ]

        for error in errors {
            let description = error.localizedDescription
            #expect(!description.isEmpty, "Error should have description: \(error)")
        }
    }

    // MARK: - OperationOptions Tests

    @Test("OperationOptions timeout conversion")
    func testOperationOptionsTimeout() {
        let options = OperationOptions(timeout: .seconds(30))
        #expect(options.timeoutMs == 30_000)

        let noTimeout = OperationOptions()
        #expect(noTimeout.timeoutMs == 0)
    }
}
