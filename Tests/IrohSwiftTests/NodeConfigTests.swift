import Foundation
import Testing
@testable import IrohSwift

/// `IrohConfig.servesBlobs`, `IrohConfig.acceptsPushes` and `NodeInfo`.
struct NodeConfigTests {
    @Test("IrohConfig serves blobs by default")
    func servesBlobsDefault() {
        #expect(IrohConfig().servesBlobs)
        #expect(!IrohConfig(servesBlobs: false).servesBlobs)
        #expect(IrohConfig(relayEnabled: false, docsEnabled: true).servesBlobs)
    }

    @Test("IrohConfig refuses pushes by default")
    func acceptsPushesDefault() {
        #expect(!IrohConfig().acceptsPushes)
        #expect(!IrohConfig(servesBlobs: true).acceptsPushes)
        #expect(IrohConfig(servesBlobs: true, acceptsPushes: true).acceptsPushes)
    }

    @Test("acceptsPushes without servesBlobs is an invalid configuration")
    func acceptsPushesRequiresServesBlobs() async throws {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let config = IrohConfig(
            storagePath: dir, relayEnabled: false, servesBlobs: false, acceptsPushes: true
        )

        let validateError = #expect(throws: IrohError.self) { try config.validate() }
        #expect(validateError?.isInvalidConfiguration == true, "got \(String(describing: validateError))")
        // Rejected before anything is created on disk.
        #expect(!FileManager.default.fileExists(atPath: dir.path))

        let initError = await #expect(throws: IrohError.self) { try await IrohNode(config: config) }
        #expect(initError?.isInvalidConfiguration == true, "got \(String(describing: initError))")
    }

    @Test("a client-only node starts, stores blobs and reports its direct addresses")
    func clientOnlyNode() async throws {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let node = try await IrohNode(
            config: IrohConfig(storagePath: dir, relayEnabled: false, servesBlobs: false)
        )

        let data = Data("kept locally".utf8)
        let ticket = try await node.put(data)
        #expect(try await node.get(ticket: ticket) == data)

        let info = try await node.info()
        #expect(info.nodeId.count == 64)
        #expect(!info.directAddrs.isEmpty)
        for addr in info.directAddrs {
            // "ip:port" or "[ipv6]:port"
            let port = addr.split(separator: ":").last.flatMap { UInt16($0) }
            #expect(port != nil && port != 0, "not ip:port: \(addr)")
        }

        try await node.close()
    }

    @Test("info() throws nodeClosed after close")
    func infoAfterClose() async throws {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let node = try await IrohNode(config: IrohConfig(storagePath: dir, relayEnabled: false))
        try await node.close()

        let error = await #expect(throws: IrohError.self) { try await node.info() }
        #expect(error?.isNodeClosed == true)
    }
}
