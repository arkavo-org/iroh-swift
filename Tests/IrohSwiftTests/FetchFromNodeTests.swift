import Foundation
import os
import Testing
@testable import IrohSwift

/// Two in-process nodes, relays off, talking over direct addresses:
/// a serving node and a client-only node (`servesBlobs: false`).
struct FetchFromNodeTests {
    /// A node plus the directory that holds its store.
    private struct TestNode {
        let node: IrohNode
        let dir: URL

        func close() async {
            try? await node.close()
            try? FileManager.default.removeItem(at: dir)
        }
    }

    private func makeNode(servesBlobs: Bool, acceptsPushes: Bool = false) async throws -> TestNode {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let node = try await IrohNode(
            config: IrohConfig(
                storagePath: dir,
                relayEnabled: false,
                servesBlobs: servesBlobs,
                acceptsPushes: acceptsPushes
            )
        )
        return TestNode(node: node, dir: dir)
    }

    private func makeOutputDir() throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fetch-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func contents(of dir: URL) throws -> [String] {
        try FileManager.default.contentsOfDirectory(atPath: dir.path).sorted()
    }

    /// Deterministic test content.
    private func testData(count: Int) -> Data {
        Data((0..<count).map { UInt8(truncatingIfNeeded: ($0 % 251) ^ ($0 / 251)) })
    }

    /// Records progress reports from the Rust thread.
    private final class ProgressLog: Sendable {
        private let entries = OSAllocatedUnfairLock<[(UInt64, UInt64)]>(initialState: [])
        func append(_ downloaded: UInt64, _ total: UInt64) {
            entries.withLock { $0.append((downloaded, total)) }
        }
        var all: [(UInt64, UInt64)] { entries.withLock { $0 } }
    }

    @Test("client-only node fetches a blob by hash from a serving node into a file")
    func fetchRoundTrip() async throws {
        let server = try await makeNode(servesBlobs: true)
        let client = try await makeNode(servesBlobs: false)
        let out = try makeOutputDir()
        defer { try? FileManager.default.removeItem(at: out) }

        let data = testData(count: 3 * 1024 * 1024 + 123)
        let ticket = try await server.node.put(data)
        let hash = IrohBLAKE3.hex(data)
        #expect(await validateTicket(ticket).hash == hash)

        let serverInfo = try await server.node.info()
        #expect(!serverInfo.directAddrs.isEmpty)
        let destination = out.appendingPathComponent("blob.tdf")
        let log = ProgressLog()

        let size = try await client.node.fetch(
            hash: hash,
            fromNode: try await server.node.endpointID(),
            directAddrs: serverInfo.directAddrs,
            to: destination,
            maxSize: UInt64(data.count),
            progress: { log.append($0, $1) }
        )

        #expect(size == UInt64(data.count))
        #expect(try Data(contentsOf: destination) == data)
        #expect(try contents(of: out) == ["blob.tdf"])

        let progress = log.all
        let total = UInt64(data.count)
        #expect(progress.count > 2)
        #expect(progress.first.map { $0 == (0, total) } == true)
        #expect(progress.last.map { $0 == (total, total) } == true)
        #expect(zip(progress, progress.dropFirst()).allSatisfy { $0.0 <= $1.0 })
        #expect(progress.allSatisfy { $0.1 == total })

        await client.close()
        await server.close()
    }

    @Test("fetch without a progress handler; an existing destination is replaced")
    func fetchReplacesExistingFile() async throws {
        let server = try await makeNode(servesBlobs: true)
        let client = try await makeNode(servesBlobs: false)
        let out = try makeOutputDir()
        defer { try? FileManager.default.removeItem(at: out) }

        let data = Data("small blob".utf8)
        _ = try await server.node.put(data)
        let destination = out.appendingPathComponent("blob.bin")
        try Data("old contents that are longer than the blob".utf8).write(to: destination)

        let size = try await client.node.fetch(
            hash: IrohBLAKE3.hex(data),
            fromNode: try await server.node.endpointID(),
            directAddrs: try await server.node.info().directAddrs,
            to: destination,
            maxSize: 1024
        )
        #expect(size == UInt64(data.count))
        #expect(try Data(contentsOf: destination) == data)

        await client.close()
        await server.close()
    }

    @Test("a blob larger than maxSize is rejected and no file is written")
    func sizeLimitExceeded() async throws {
        let server = try await makeNode(servesBlobs: true)
        let client = try await makeNode(servesBlobs: false)
        let out = try makeOutputDir()
        defer { try? FileManager.default.removeItem(at: out) }

        let data = testData(count: 100_000)
        _ = try await server.node.put(data)
        let log = ProgressLog()

        do {
            _ = try await client.node.fetch(
                hash: IrohBLAKE3.hex(data),
                fromNode: try await server.node.endpointID(),
                directAddrs: try await server.node.info().directAddrs,
                to: out.appendingPathComponent("blob.bin"),
                maxSize: UInt64(data.count - 1),
                progress: { log.append($0, $1) }
            )
            Issue.record("expected sizeLimitExceeded")
        } catch IrohError.sizeLimitExceeded(let size) {
            #expect(size == UInt64(data.count))
        }
        #expect(log.all.isEmpty)
        #expect(try contents(of: out).isEmpty)

        await client.close()
        await server.close()
    }

    @Test("an unknown hash throws notFound and leaves an existing destination untouched")
    func unknownHashIsNotFound() async throws {
        let server = try await makeNode(servesBlobs: true)
        let client = try await makeNode(servesBlobs: false)
        let out = try makeOutputDir()
        defer { try? FileManager.default.removeItem(at: out) }

        let destination = out.appendingPathComponent("blob.bin")
        let previous = Data("previous".utf8)
        try previous.write(to: destination)

        let error = await #expect(throws: IrohError.self) {
            try await client.node.fetch(
                hash: IrohBLAKE3.hex(Data("never stored".utf8)),
                fromNode: try await server.node.endpointID(),
                directAddrs: try await server.node.info().directAddrs,
                to: destination,
                maxSize: .max
            )
        }
        #expect(error?.isNotFound == true, "got \(String(describing: error))")
        #expect(try Data(contentsOf: destination) == previous)
        #expect(try contents(of: out) == ["blob.bin"])

        await client.close()
        await server.close()
    }

    @Test("a client-only node refuses to serve blobs")
    func clientOnlyNodeDoesNotServe() async throws {
        let fetcher = try await makeNode(servesBlobs: true)
        let clientOnly = try await makeNode(servesBlobs: false)
        let out = try makeOutputDir()
        defer { try? FileManager.default.removeItem(at: out) }

        let data = Data("held but not served".utf8)
        _ = try await clientOnly.node.put(data)

        do {
            _ = try await fetcher.node.fetch(
                hash: IrohBLAKE3.hex(data),
                fromNode: try await clientOnly.node.endpointID(),
                directAddrs: try await clientOnly.node.info().directAddrs,
                to: out.appendingPathComponent("blob.bin"),
                maxSize: .max
            )
            Issue.record("expected the fetch to fail")
        } catch IrohError.fetchFailed {
            // Refused at the connection handshake (no blobs ALPN).
        }
        #expect(try contents(of: out).isEmpty)

        await clientOnly.close()
        await fetcher.close()
    }

    @Test("cancelling mid-transfer throws CancellationError and leaves no file")
    func cancelMidTransfer() async throws {
        let server = try await makeNode(servesBlobs: true)
        let client = try await makeNode(servesBlobs: false)
        let out = try makeOutputDir()
        defer { try? FileManager.default.removeItem(at: out) }

        let data = testData(count: 8 * 1024 * 1024)
        _ = try await server.node.put(data)
        let hash = IrohBLAKE3.hex(data)
        let serverID = try await server.node.endpointID()
        let addrs = try await server.node.info().directAddrs
        let destination = out.appendingPathComponent("blob.bin")

        // The first content progress report parks the transfer until the test
        // has cancelled the task, so the cancel always lands mid-transfer.
        let (started, startedContinuation) = AsyncStream<Void>.makeStream()
        let resume = DispatchSemaphore(value: 0)
        let reported = OSAllocatedUnfairLock(initialState: false)
        let log = ProgressLog()

        let task = Task {
            try await client.node.fetch(
                hash: hash,
                fromNode: serverID,
                directAddrs: addrs,
                to: destination,
                maxSize: .max,
                progress: { downloaded, total in
                    log.append(downloaded, total)
                    guard downloaded > 0 else { return }
                    let first = reported.withLock { done in
                        defer { done = true }
                        return !done
                    }
                    if first {
                        startedContinuation.yield()
                        _ = resume.wait(timeout: .now() + 10)
                    }
                }
            )
        }

        var iterator = started.makeAsyncIterator()
        _ = await iterator.next()
        task.cancel()
        resume.signal()

        await #expect(throws: CancellationError.self) { try await task.value }
        let lastDownloaded = log.all.last?.0 ?? 0
        #expect(lastDownloaded < UInt64(data.count))
        #expect(try contents(of: out).isEmpty)

        await client.close()
        await server.close()
    }

    @Test("cancelling while connecting throws CancellationError promptly")
    func cancelWhileConnecting() async throws {
        let absent = try await makeNode(servesBlobs: true)
        let absentID = try await absent.node.endpointID()
        await absent.close()
        let client = try await makeNode(servesBlobs: false)
        let out = try makeOutputDir()
        defer { try? FileManager.default.removeItem(at: out) }

        let started = ContinuousClock.now
        let task = Task {
            try await client.node.fetch(
                hash: IrohBLAKE3.hex(Data("x".utf8)),
                fromNode: absentID,
                directAddrs: ["127.0.0.1:9"],
                to: out.appendingPathComponent("blob.bin"),
                maxSize: .max
            )
        }
        try await Task.sleep(for: .milliseconds(200))
        task.cancel()

        await #expect(throws: CancellationError.self) { try await task.value }
        #expect(ContinuousClock.now - started < .seconds(5))
        #expect(try contents(of: out).isEmpty)

        await client.close()
    }

    @Test("an already-cancelled task does not start a fetch")
    func alreadyCancelled() async throws {
        let client = try await makeNode(servesBlobs: false)
        let out = try makeOutputDir()
        defer { try? FileManager.default.removeItem(at: out) }
        let clientID = try await client.node.endpointID()

        let task = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            return try await client.node.fetch(
                hash: IrohBLAKE3.hex(Data()),
                fromNode: clientID,
                to: out.appendingPathComponent("blob.bin"),
                maxSize: .max
            )
        }
        await #expect(throws: CancellationError.self) { try await task.value }
        #expect(try contents(of: out).isEmpty)

        await client.close()
    }

    @Test("closing the node fails an in-flight fetch with nodeClosed")
    func closeDuringFetch() async throws {
        let absent = try await makeNode(servesBlobs: true)
        let absentID = try await absent.node.endpointID()
        await absent.close()
        let client = try await makeNode(servesBlobs: false)
        let out = try makeOutputDir()
        defer { try? FileManager.default.removeItem(at: out) }

        let task = Task {
            try await client.node.fetch(
                hash: IrohBLAKE3.hex(Data("x".utf8)),
                fromNode: absentID,
                directAddrs: ["127.0.0.1:9"],
                to: out.appendingPathComponent("blob.bin"),
                maxSize: .max
            )
        }
        try await Task.sleep(for: .milliseconds(200))
        try await client.node.close()

        let error = await #expect(throws: IrohError.self) { try await task.value }
        #expect(error?.isNodeClosed == true, "got \(String(describing: error))")
        #expect(try contents(of: out).isEmpty)
        try? FileManager.default.removeItem(at: client.dir)
    }

    @Test("invalid arguments throw fetchFailed")
    func invalidArguments() async throws {
        let client = try await makeNode(servesBlobs: false)
        let out = try makeOutputDir()
        defer { try? FileManager.default.removeItem(at: out) }
        let clientID = try await client.node.endpointID()
        let destination = out.appendingPathComponent("blob.bin")

        let badHash = await #expect(throws: IrohError.self) {
            try await client.node.fetch(hash: "not-a-hash", fromNode: clientID, to: destination, maxSize: 1)
        }
        #expect(badHash?.isFetchFailed == true)
        let badNode = await #expect(throws: IrohError.self) {
            try await client.node.fetch(
                hash: IrohBLAKE3.hex(Data()), fromNode: "not-a-node-id", to: destination, maxSize: 1
            )
        }
        #expect(badNode?.isFetchFailed == true)
        let badURL = await #expect(throws: IrohError.self) {
            try await client.node.fetch(
                hash: IrohBLAKE3.hex(Data()),
                fromNode: clientID,
                to: URL(string: "https://example.com/blob")!,
                maxSize: 1
            )
        }
        #expect(badURL?.isFetchFailed == true)
        let badAddr = await #expect(throws: IrohError.self) {
            try await client.node.fetch(
                hash: IrohBLAKE3.hex(Data()),
                fromNode: clientID,
                directAddrs: ["not an address"],
                to: destination,
                maxSize: 1
            )
        }
        #expect(badAddr?.isFetchFailed == true)
        #expect(try contents(of: out).isEmpty)

        await client.close()
    }

    @Test("client-only node pushes Data to a serving node and fetches it back")
    func pushDataThenFetch() async throws {
        let server = try await makeNode(servesBlobs: true, acceptsPushes: true)
        let client = try await makeNode(servesBlobs: false)
        let out = try makeOutputDir()
        defer { try? FileManager.default.removeItem(at: out) }

        let serverID = try await server.node.endpointID()
        let addrs = try await server.node.info().directAddrs
        let data = testData(count: 200_000)

        let hash = try await client.node.pushToNode(nodeId: serverID, directAddrs: addrs, data: data)
        #expect(hash == IrohBLAKE3.hex(data))

        let destination = out.appendingPathComponent("back.bin")
        let size = try await client.node.fetch(
            hash: hash, fromNode: serverID, directAddrs: addrs, to: destination, maxSize: .max
        )
        #expect(size == UInt64(data.count))
        #expect(try Data(contentsOf: destination) == data)

        await client.close()
        await server.close()
    }

    @Test("client-only node pushes a file to a serving node and fetches it back")
    func pushFileThenFetch() async throws {
        let server = try await makeNode(servesBlobs: true, acceptsPushes: true)
        let client = try await makeNode(servesBlobs: false)
        let out = try makeOutputDir()
        defer { try? FileManager.default.removeItem(at: out) }

        let serverID = try await server.node.endpointID()
        let addrs = try await server.node.info().directAddrs
        let data = testData(count: 2 * 1024 * 1024 + 5)
        let source = out.appendingPathComponent("source.tdf")
        try data.write(to: source)

        let hash = try await client.node.pushToNode(nodeId: serverID, directAddrs: addrs, fileURL: source)
        #expect(hash == IrohBLAKE3.hex(data))
        // The source file is untouched.
        #expect(try Data(contentsOf: source) == data)

        let destination = out.appendingPathComponent("fetched.tdf")
        let size = try await client.node.fetch(
            hash: hash, fromNode: serverID, directAddrs: addrs, to: destination, maxSize: .max
        )
        #expect(size == UInt64(data.count))
        #expect(try Data(contentsOf: destination) == data)
        #expect(IrohBLAKE3.hex(try Data(contentsOf: destination)) == hash)

        await client.close()
        await server.close()
    }

    @Test("a serving node refuses pushes unless acceptsPushes is set")
    func servingNodeRefusesPushesByDefault() async throws {
        let server = try await makeNode(servesBlobs: true)
        let client = try await makeNode(servesBlobs: false)
        let out = try makeOutputDir()
        defer { try? FileManager.default.removeItem(at: out) }

        let serverID = try await server.node.endpointID()
        let addrs = try await server.node.info().directAddrs

        // A small refused push can still report success (the pusher does not
        // see the refusal), so its outcome is not part of the contract.
        let small = Data("refused push".utf8)
        _ = try? await client.node.pushToNode(nodeId: serverID, directAddrs: addrs, data: small)

        // A larger one fails: the server stops the stream while it is being written.
        let large = testData(count: 2 * 1024 * 1024 + 3)
        let source = out.appendingPathComponent("large.bin")
        try large.write(to: source)
        let pushError = await #expect(throws: IrohError.self) {
            try await client.node.pushToNode(nodeId: serverID, directAddrs: addrs, fileURL: source)
        }
        #expect(pushError?.isPushFailed == true, "got \(String(describing: pushError))")

        // Neither blob reached the server's store.
        for data in [small, large] {
            let error = await #expect(throws: IrohError.self) {
                try await client.node.fetch(
                    hash: IrohBLAKE3.hex(data),
                    fromNode: serverID,
                    directAddrs: addrs,
                    to: out.appendingPathComponent("fetched.bin"),
                    maxSize: .max
                )
            }
            #expect(error?.isNotFound == true, "got \(String(describing: error))")
        }

        await client.close()
        await server.close()
    }

    /// Port of the node's first IPv4 direct address (`"ip:port"`).
    private func ipv4Port(of node: IrohNode) async throws -> String {
        let addrs = try await node.info().directAddrs
        let v4 = try #require(addrs.first { !$0.hasPrefix("[") })
        return String(try #require(v4.split(separator: ":").last))
    }

    @Test("fetch and push accept host:port direct addresses")
    func hostnameDirectAddress() async throws {
        let server = try await makeNode(servesBlobs: true, acceptsPushes: true)
        let client = try await makeNode(servesBlobs: false)
        let out = try makeOutputDir()
        defer { try? FileManager.default.removeItem(at: out) }

        let serverID = try await server.node.endpointID()
        let hostAddr = "localhost:\(try await ipv4Port(of: server.node))"

        let data = testData(count: 200_000)
        let hash = IrohBLAKE3.hex(data)
        _ = try await server.node.put(data)
        let destination = out.appendingPathComponent("by-host.bin")
        let size = try await client.node.fetch(
            hash: hash,
            fromNode: serverID,
            directAddrs: [hostAddr],
            to: destination,
            maxSize: .max
        )
        #expect(size == UInt64(data.count))
        #expect(try Data(contentsOf: destination) == data)

        let pushed = testData(count: 1_000)
        let pushedHash = try await client.node.pushToNode(
            nodeId: serverID, directAddrs: [hostAddr], data: pushed
        )
        #expect(pushedHash == IrohBLAKE3.hex(pushed))

        await client.close()
        await server.close()
    }

    @Test("a host that does not resolve, or a malformed address, fails cleanly")
    func badHostDirectAddress() async throws {
        let server = try await makeNode(servesBlobs: true, acceptsPushes: true)
        let client = try await makeNode(servesBlobs: false)
        let out = try makeOutputDir()
        defer { try? FileManager.default.removeItem(at: out) }
        let serverID = try await server.node.endpointID()
        let destination = out.appendingPathComponent("never.bin")

        for bad in ["no-such-host.invalid:11204", "host-without-port", "host:0", "::1"] {
            let fetchError = await #expect(throws: IrohError.self) {
                try await client.node.fetch(
                    hash: IrohBLAKE3.hex(Data("x".utf8)),
                    fromNode: serverID,
                    directAddrs: [bad],
                    to: destination,
                    maxSize: .max
                )
            }
            #expect(fetchError?.isFetchFailed == true, "\(bad): \(String(describing: fetchError))")

            let pushError = await #expect(throws: IrohError.self) {
                try await client.node.pushToNode(
                    nodeId: serverID, directAddrs: [bad], data: Data("x".utf8)
                )
            }
            #expect(pushError?.isPushFailed == true, "\(bad): \(String(describing: pushError))")
        }
        #expect(!FileManager.default.fileExists(atPath: destination.path))

        await client.close()
        await server.close()
    }

    @Test("a Data push to an unreachable address hits its time limit, and can be cancelled")
    func dataPushTimeoutAndCancel() async throws {
        let server = try await makeNode(servesBlobs: true, acceptsPushes: true)
        let client = try await makeNode(servesBlobs: false)
        let serverID = try await server.node.endpointID()
        let blackhole = ["192.0.2.1:9"]  // TEST-NET-1: nothing answers

        let timeoutError = await #expect(throws: IrohError.self) {
            try await client.node.pushToNode(
                nodeId: serverID,
                directAddrs: blackhole,
                data: Data("x".utf8),
                options: OperationOptions(timeout: .milliseconds(500))
            )
        }
        if case .timeout? = timeoutError {} else {
            Issue.record("expected .timeout, got \(String(describing: timeoutError))")
        }

        let node = client.node
        let task = Task {
            try await node.pushToNode(nodeId: serverID, directAddrs: blackhole, data: Data("x".utf8))
        }
        try await Task.sleep(for: .milliseconds(300))
        task.cancel()
        let result = await task.result
        if case .failure(let error) = result, error is CancellationError {} else {
            Issue.record("expected CancellationError, got \(result)")
        }

        await client.close()
        await server.close()
    }

    @Test("pushToNode(fileURL:) with a missing file throws pushFailed")
    func pushMissingFile() async throws {
        let server = try await makeNode(servesBlobs: true)
        let client = try await makeNode(servesBlobs: false)

        let missing = FileManager.default.temporaryDirectory
            .appendingPathComponent("missing-\(UUID().uuidString).tdf")
        let error = await #expect(throws: IrohError.self) {
            try await client.node.pushToNode(
                nodeId: try await server.node.endpointID(),
                directAddrs: try await server.node.info().directAddrs,
                fileURL: missing
            )
        }
        #expect(error?.isPushFailed == true, "got \(String(describing: error))")

        await client.close()
        await server.close()
    }
}
