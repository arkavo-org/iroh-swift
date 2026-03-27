import Testing
import Foundation
@testable import IrohSwift

struct PushToNodeTests {
    @Test("pushToNode returns valid hash hex string")
    func testPushToNodeReturnsHash() async throws {
        let configA = IrohConfig(relayEnabled: false)
        let configB = IrohConfig(relayEnabled: false)
        let nodeA = try await IrohNode(config: configA)
        let nodeB = try await IrohNode(config: configB)

        let nodeInfoB = try await nodeB.info()
        let nodeBId = nodeInfoB.nodeId

        let data = "Push test from Swift!".data(using: .utf8)!
        let hashHex = try await nodeA.pushToNode(nodeId: nodeBId, data: data)

        #expect(hashHex.count == 64)
        #expect(hashHex.allSatisfy { $0.isHexDigit })

        try await nodeA.close()
        try await nodeB.close()
    }

    @Test("pushToNode with invalid node ID throws pushFailed")
    func testPushToNodeInvalidNodeId() async throws {
        let config = IrohConfig(relayEnabled: false)
        let node = try await IrohNode(config: config)

        let data = "test".data(using: .utf8)!

        await #expect(throws: IrohError.self) {
            try await node.pushToNode(nodeId: "not-a-valid-hex-node-id", data: data)
        }

        try await node.close()
    }

    @Test("pushToNode on closed node throws nodeClosed")
    func testPushToNodeClosedNode() async throws {
        let config = IrohConfig(relayEnabled: false)
        let node = try await IrohNode(config: config)
        try await node.close()

        let data = "test".data(using: .utf8)!

        await #expect(throws: IrohError.self) {
            try await node.pushToNode(nodeId: "deadbeef", data: data)
        }
    }
}
