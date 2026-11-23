import Foundation
import Testing
@testable import IrohSwift

/// Regression tests covering the 16 audit findings from issue #25.
struct AuditRegressionTests {
    private func makeTempDir(_ prefix: String = "iroh-audit") -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent("\(prefix)-\(UUID().uuidString)", isDirectory: true)
    }

    // MARK: - #3 OperationOptions.timeoutMs edge cases

    @Test("OperationOptions.timeoutMs handles nil, zero, negative, sub-ms, and huge durations without trapping")
    func operationOptionsTimeoutEdgeCases() {
        let noTimeout = OperationOptions()
        #expect(noTimeout.timeoutMs == 0)
        #expect(!noTimeout.isImmediatelyExpired)

        let zero = OperationOptions(timeout: .zero)
        #expect(zero.timeoutMs == 0)
        #expect(zero.isImmediatelyExpired)

        for neg in [Duration.seconds(-1), Duration.milliseconds(-500), Duration.nanoseconds(-1)] {
            let opt = OperationOptions(timeout: neg)
            #expect(opt.timeoutMs == 0)
            #expect(opt.isImmediatelyExpired)
        }

        for subMs in [Duration.nanoseconds(1), Duration.microseconds(500)] {
            let opt = OperationOptions(timeout: subMs)
            #expect(opt.timeoutMs == 1)
            #expect(!opt.isImmediatelyExpired)
        }

        let huge = OperationOptions(timeout: Duration(secondsComponent: Int64.max, attosecondsComponent: 0))
        #expect(huge.timeoutMs == UInt64.max)
        #expect(!huge.isImmediatelyExpired)

        let normal = OperationOptions(timeout: .milliseconds(2_500))
        #expect(normal.timeoutMs == 2_500)
        #expect(!normal.isImmediatelyExpired)
    }

    @Test("Zero and negative timeouts fail immediately with IrohError.timeout on put, get, and pushToNode")
    func expiredTimeoutsFailImmediately() async throws {
        let dir = makeTempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let node = try await IrohNode(config: IrohConfig(storagePath: dir, relayEnabled: false))
        let ticket = try await node.put(Data("hello".utf8))
        let nodeId = try await node.endpointID()

        for expired in [OperationOptions(timeout: .zero), OperationOptions(timeout: .seconds(-5))] {
            let putErr = await #expect(throws: IrohError.self) {
                _ = try await node.put(Data("x".utf8), options: expired)
            }
            if case .timeout = putErr { } else { Issue.record("expected timeout, got \(String(describing: putErr))") }

            let getErr = await #expect(throws: IrohError.self) {
                _ = try await node.get(ticket: ticket, options: expired)
            }
            if case .timeout = getErr { } else { Issue.record("expected timeout, got \(String(describing: getErr))") }

            let pushErr = await #expect(throws: IrohError.self) {
                _ = try await node.pushToNode(nodeId: nodeId, data: Data("x".utf8), options: expired)
            }
            if case .timeout = pushErr { } else { Issue.record("expected timeout, got \(String(describing: pushErr))") }
        }

        try await node.close()
    }

    // MARK: - #2 & #5 maxSize enforcement & BlobTicket direct IP hints via MemoryLookup

    @Test("Cross-node get(ticket:) succeeds with relayEnabled: false via MemoryLookup and enforces maxSize")
    func crossNodeGetTicketAndMaxSize() async throws {
        let serverDir = makeTempDir("server")
        let clientDir = makeTempDir("client")
        defer {
            try? FileManager.default.removeItem(at: serverDir)
            try? FileManager.default.removeItem(at: clientDir)
        }

        let server = try await IrohNode(config: IrohConfig(storagePath: serverDir, relayEnabled: false, servesBlobs: true))
        let client = try await IrohNode(config: IrohConfig(storagePath: clientDir, relayEnabled: false, servesBlobs: false))

        let payload = Data((0..<(128 * 1024)).map { UInt8(truncatingIfNeeded: $0) })
        let ticket = try await server.put(payload)

        // maxSize smaller than payload rejects before downloading.
        let sizeErr = await #expect(throws: IrohError.self) {
            _ = try await client.get(ticket: ticket, options: OperationOptions(maxSize: 1024))
        }
        if case .sizeLimitExceeded(let announced) = sizeErr {
            #expect(announced == UInt64(payload.count))
        } else {
            Issue.record("expected sizeLimitExceeded, got \(String(describing: sizeErr))")
        }

        // Exact maxSize succeeds via MemoryLookup direct IP hints (relayEnabled: false).
        let fetched = try await client.get(
            ticket: ticket,
            options: OperationOptions(maxSize: UInt64(payload.count))
        )
        #expect(fetched == payload)

        // Empty blob zero-copy path is safe.
        let emptyTicket = try await server.put(Data())
        let emptyFetched = try await client.get(ticket: emptyTicket, options: OperationOptions(maxSize: 0))
        #expect(emptyFetched.isEmpty)

        try await client.close()
        try await server.close()
    }

    // MARK: - #1 & #15 Non-blocking async FFI & cancellation

    @Test("Pre-cancelled IrohNode.init throws CancellationError without touching disk")
    func preCancelledInitDoesNotTouchDisk() async {
        let dir = makeTempDir("precancel")
        defer { try? FileManager.default.removeItem(at: dir) }

        let task = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            return try await IrohNode(config: IrohConfig(storagePath: dir, relayEnabled: false))
        }

        await #expect(throws: CancellationError.self) {
            let node = try await task.value
            try? await node.close()
        }
        #expect(!FileManager.default.fileExists(atPath: dir.path))
    }

    @Test("In-flight get(ticket:) does not freeze IrohNode actor and stops on task cancellation")
    func inFlightGetDoesNotFreezeActorAndCancels() async throws {
        let serverDir = makeTempDir("stopped-server")
        let clientDir = makeTempDir("cancel-client")
        defer {
            try? FileManager.default.removeItem(at: serverDir)
            try? FileManager.default.removeItem(at: clientDir)
        }

        // Mint a real ticket pointing at a closed peer so dialing hangs/retries.
        let server = try await IrohNode(config: IrohConfig(storagePath: serverDir, relayEnabled: false))
        let ticket = try await server.put(Data("unreachable".utf8))
        try await server.close()

        let client = try await IrohNode(config: IrohConfig(storagePath: clientDir, relayEnabled: false, servesBlobs: false))

        let getTask = Task {
            try await client.get(ticket: ticket)
        }

        // Give the get task time to enter Rust; actor must remain responsive to info().
        try await Task.sleep(for: .milliseconds(50))
        let info = try await client.info()
        #expect(info.nodeId.count == 64)

        // Cancelling the get task must abort promptly.
        let cancelStart = ContinuousClock.now
        getTask.cancel()
        await #expect(throws: CancellationError.self) {
            _ = try await getTask.value
        }
        #expect(ContinuousClock.now - cancelStart < .seconds(2))

        // Short timeout against the same unreachable peer reports structured IrohError.timeout.
        let timeoutErr = await #expect(throws: IrohError.self) {
            _ = try await client.get(ticket: ticket, options: OperationOptions(timeout: .milliseconds(200)))
        }
        if case .timeout = timeoutErr { } else { Issue.record("expected timeout, got \(String(describing: timeoutErr))") }

        try await client.close()
    }

    // MARK: - #4 Client-only node FsStore shutdown

    @Test("Client-only node (servesBlobs: false) flushes FsStore on close and reopens cleanly")
    func clientOnlyShutdownFlushesStore() async throws {
        let dir = makeTempDir("client-only-flush")
        defer { try? FileManager.default.removeItem(at: dir) }

        let payload = Data("persisted-on-client-only".utf8)
        let node1 = try await IrohNode(config: IrohConfig(storagePath: dir, relayEnabled: false, servesBlobs: false))
        let ticket = try await node1.put(payload)
        try await node1.close()

        let node2 = try await IrohNode(config: IrohConfig(storagePath: dir, relayEnabled: false, servesBlobs: false))
        let restored = try await node2.get(ticket: ticket)
        #expect(restored == payload)
        try await node2.close()
    }

    // MARK: - #7 Concurrent close() coalescing

    @Test("Concurrent close() callers coalesce and wait until shutdown finishes")
    func concurrentCloseCoalesces() async throws {
        let dir = makeTempDir("concurrent-close")
        defer { try? FileManager.default.removeItem(at: dir) }

        let node = try await IrohNode(config: IrohConfig(storagePath: dir, relayEnabled: false))
        try await withThrowingTaskGroup(of: Void.self) { group in
            for _ in 0..<4 {
                group.addTask {
                    try await node.close()
                }
            }
            try await group.waitForAll()
        }

        let err = await #expect(throws: IrohError.self) {
            _ = try await node.info()
        }
        #expect(err?.isNodeClosed == true)
    }

    // MARK: - #6 IrohNodeManager & getForUI progress + reset/init races

    @Test("getForUI delivers ordered progress on MainActor and stops after completion")
    @MainActor
    func getForUIProgressOrdering() async throws {
        let serverDir = makeTempDir("ui-server")
        let clientDir = makeTempDir("ui-client")
        defer {
            try? FileManager.default.removeItem(at: serverDir)
            try? FileManager.default.removeItem(at: clientDir)
        }

        let server = try await IrohNode(config: IrohConfig(storagePath: serverDir, relayEnabled: false))
        let client = try await IrohNode(config: IrohConfig(storagePath: clientDir, relayEnabled: false, servesBlobs: false))

        let payload = Data((0..<(512 * 1024)).map { UInt8(truncatingIfNeeded: $0) })
        let ticket = try await server.put(payload)

        @MainActor
        final class ProgressTracker {
            var updates: [DownloadProgress] = []
            var completed = false
            var lateUpdate = false
        }
        let tracker = ProgressTracker()
        let fetched = try await client.getForUI(
            ticket: ticket,
            options: OperationOptions(timeout: .seconds(10), maxSize: UInt64(payload.count))
        ) { progress in
            if tracker.completed {
                tracker.lateUpdate = true
            }
            tracker.updates.append(progress)
        }
        tracker.completed = true
        try await Task.sleep(for: .milliseconds(50))

        #expect(fetched == payload)
        #expect(!tracker.lateUpdate)
        #expect(!tracker.updates.isEmpty)
        #expect(zip(tracker.updates, tracker.updates.dropFirst()).allSatisfy { $0.downloaded <= $1.downloaded })
        #expect(tracker.updates.last?.downloaded == UInt64(payload.count))

        try await client.close()
        try await server.close()
    }

    #if canImport(SwiftUI) && canImport(Observation)
    @Test("IrohNodeManager never sticks isDownloading == true and handles reset/re-init without redb conflict")
    @MainActor
    func nodeManagerProgressAndResetReinit() async throws {
        let serverDir = makeTempDir("mgr-server")
        let clientDir = makeTempDir("mgr-client")
        defer {
            try? FileManager.default.removeItem(at: serverDir)
            try? FileManager.default.removeItem(at: clientDir)
        }

        let server = try await IrohNode(config: IrohConfig(storagePath: serverDir, relayEnabled: false))
        let payload = Data((0..<(512 * 1024)).map { UInt8(truncatingIfNeeded: $0) })
        let ticket = try await server.put(payload)

        let manager = IrohNodeManager()
        let config = IrohConfig(storagePath: clientDir, relayEnabled: false, servesBlobs: false)

        await manager.initialize(config: config)
        #expect(manager.node != nil)

        // Download with progress: isDownloading must be false immediately after and stay false.
        let downloaded = try await manager.download(ticket: ticket)
        #expect(downloaded == payload)
        #expect(!manager.isDownloading)
        try await Task.sleep(for: .milliseconds(50))
        #expect(!manager.isDownloading)

        // Failing download (sizeLimitExceeded) must also clear isDownloading permanently.
        await #expect(throws: IrohError.self) {
            _ = try await manager.download(ticket: ticket, options: OperationOptions(maxSize: 10))
        }
        #expect(!manager.isDownloading)
        try await Task.sleep(for: .milliseconds(50))
        #expect(!manager.isDownloading)

        // Reset and immediately re-initialize on the exact same storagePath (tests redb lock serialization).
        manager.reset()
        #expect(manager.node == nil)
        await manager.initialize(config: config)
        #expect(manager.node != nil)
        #expect(manager.error == nil)

        // Reset mid-initialization and re-initialize cleanly.
        manager.reset()
        let initTask = Task { @MainActor in
            await manager.initialize(config: config)
        }
        await Task.yield()
        manager.reset()
        await initTask.value
        await manager.initialize(config: config)
        #expect(manager.node != nil)
        #expect(manager.error == nil)

        // Cross-instance reset/re-init (ArkavoKit pattern: discard old manager after reset(),
        // immediately create a fresh IrohNodeManager on the exact same storagePath).
        manager.reset()
        let replacement = IrohNodeManager()
        await replacement.initialize(config: config)
        #expect(replacement.node != nil)
        #expect(replacement.error == nil)

        await replacement.resetAndWait()
        try await server.close()
    }
    #endif

    // MARK: - #10 IrohConfig.validate() fixes

    @Test("IrohConfig.validate checks isFileURL and relay host before touching disk and handles concurrent calls")
    func configValidateHardening() async throws {
        // Non-file URL rejected.
        let httpStorage = URL(string: "https://example.com/iroh")!
        let badStorage = IrohConfig(storagePath: httpStorage)
        let storageErr = #expect(throws: IrohError.self) { try badStorage.validate() }
        #expect(storageErr?.isInvalidConfiguration == true)

        // Relay URL without host rejected BEFORE creating storagePath directory.
        for badRelay in ["https://", "http://", "https:///path"] {
            let dir = makeTempDir("no-disk-side-effect")
            defer { try? FileManager.default.removeItem(at: dir) }
            let cfg = IrohConfig(storagePath: dir, customRelayUrl: badRelay)
            let err = #expect(throws: IrohError.self) { try cfg.validate() }
            #expect(err?.isInvalidConfiguration == true)
            #expect(!FileManager.default.fileExists(atPath: dir.path))
        }

        // Regular file at storagePath rejected.
        let fileURL = makeTempDir("file-not-dir")
        try Data("not a dir".utf8).write(to: fileURL)
        defer { try? FileManager.default.removeItem(at: fileURL) }
        let fileCfg = IrohConfig(storagePath: fileURL)
        let fileErr = #expect(throws: IrohError.self) { try fileCfg.validate() }
        #expect(fileErr?.isInvalidConfiguration == true)

        // Concurrent validate() on the same directory must not race on probe file.
        let sharedDir = makeTempDir("concurrent-validate")
        defer { try? FileManager.default.removeItem(at: sharedDir) }
        let validCfg = IrohConfig(storagePath: sharedDir, relayEnabled: false)
        try await withThrowingTaskGroup(of: Void.self) { group in
            for _ in 0..<16 {
                group.addTask {
                    try validCfg.validate()
                }
            }
            try await group.waitForAll()
        }
    }

    // MARK: - #11 Push short timeout excludes post-transfer grace sleep

    @Test("pushToNode with 250ms timeout succeeds on localhost without false timeout during grace sleep")
    func pushShortTimeoutSucceeds() async throws {
        let senderDir = makeTempDir("push-sender")
        let receiverDir = makeTempDir("push-receiver")
        defer {
            try? FileManager.default.removeItem(at: senderDir)
            try? FileManager.default.removeItem(at: receiverDir)
        }

        let sender = try await IrohNode(config: IrohConfig(storagePath: senderDir, relayEnabled: false, servesBlobs: false))
        let receiver = try await IrohNode(config: IrohConfig(storagePath: receiverDir, relayEnabled: false, servesBlobs: true, acceptsPushes: true))
        let info = try await receiver.info()

        let data = Data("short timeout push".utf8)
        let hash = try await sender.pushToNode(
            nodeId: info.nodeId,
            directAddrs: info.directAddrs,
            data: data,
            options: OperationOptions(timeout: .milliseconds(250))
        )
        #expect(hash == IrohBLAKE3.hex(data))

        try await sender.close()
        try await receiver.close()
    }

    // MARK: - #12 Exponential backoff overflow cap

    @Test("nextBackoffDelay caps cleanly without overflowing Duration")
    func backoffNeverOverflows() {
        var delay: Duration = .seconds(1)
        for _ in 0..<100 {
            delay = IrohNode.nextBackoffDelay(after: delay, maxDelay: .seconds(60))
        }
        #expect(delay == .seconds(60))

        let huge = Duration(secondsComponent: Int64.max, attosecondsComponent: 0)
        #expect(IrohNode.nextBackoffDelay(after: huge, maxDelay: .seconds(60)) == huge)

        #expect(IrohNode.nextBackoffDelay(after: .seconds(-5), maxDelay: .seconds(60)) == .zero)
    }
}
