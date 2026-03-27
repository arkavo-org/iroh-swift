import Testing
import Foundation
@testable import IrohSwift

struct PushToNodeTests {
    @Test("pushToNode with invalid node ID throws pushFailed")
    func testPushToNodeInvalidNodeId() async throws {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let config = IrohConfig(storagePath: tempDir, relayEnabled: false)
        let node = try await IrohNode(config: config)

        let data = "test".data(using: .utf8)!

        await #expect(throws: IrohError.self) {
            try await node.pushToNode(nodeId: "not-a-valid-hex-node-id", data: data)
        }

        try await node.close()
    }

    @Test("pushToNode on closed node throws nodeClosed")
    func testPushToNodeClosedNode() async throws {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let config = IrohConfig(storagePath: tempDir, relayEnabled: false)
        let node = try await IrohNode(config: config)
        try await node.close()

        let data = "test".data(using: .utf8)!

        await #expect(throws: IrohError.self) {
            try await node.pushToNode(nodeId: "deadbeef", data: data)
        }
    }
}
