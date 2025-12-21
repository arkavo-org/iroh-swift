#if canImport(SwiftUI) && canImport(Observation)
import XCTest
@testable import IrohSwift

final class IrohNodeManagerTests: XCTestCase {

    @MainActor
    func testInitialState() {
        let manager = IrohNodeManager()
        XCTAssertNil(manager.node)
        XCTAssertFalse(manager.isInitializing)
        XCTAssertNil(manager.error)
    }

    @MainActor
    func testSuccessfulInitialization() async {
        let manager = IrohNodeManager()
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let config = IrohConfig(storagePath: tempDir, relayEnabled: false)

        await manager.initialize(config: config)

        XCTAssertNotNil(manager.node)
        XCTAssertFalse(manager.isInitializing)
        XCTAssertNil(manager.error)
    }

    @MainActor
    func testReset() async {
        let manager = IrohNodeManager()
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let config = IrohConfig(storagePath: tempDir, relayEnabled: false)

        await manager.initialize(config: config)
        manager.reset()

        XCTAssertNil(manager.node)
        XCTAssertFalse(manager.isInitializing)
        XCTAssertNil(manager.error)
    }

    @MainActor
    func testMultipleInitializationsIgnored() async {
        let manager = IrohNodeManager()
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let config = IrohConfig(storagePath: tempDir, relayEnabled: false)

        await manager.initialize(config: config)
        let firstNode = manager.node

        // Second initialization should be ignored
        await manager.initialize(config: config)
        XCTAssertTrue(manager.node === firstNode)
    }
}
#endif
