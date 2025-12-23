import XCTest
@testable import IrohSwift

final class IrohDocTests: XCTestCase {
    private var tempDir: URL!
    private var node: IrohNode!
    private var author: IrohAuthor!

    override func setUp() async throws {
        tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("iroh-doc-tests-\(UUID().uuidString)", isDirectory: true)

        let config = IrohConfig(
            storagePath: tempDir,
            relayEnabled: false,
            docsEnabled: true
        )
        node = try await IrohNode(config: config)
        author = try await IrohAuthor.create()
        try await node.importAuthor(author)
    }

    override func tearDown() async throws {
        try? FileManager.default.removeItem(at: tempDir)
    }

    /// Test creating a document returns a valid namespace ID.
    func testDocCreation() async throws {
        let doc = try await node.createDoc()
        let namespaceId = await doc.namespaceId

        XCTAssertFalse(namespaceId.isEmpty, "Namespace ID should not be empty")
        XCTAssertEqual(namespaceId.count, 64, "Namespace ID should be 64 characters (hex)")
    }

    /// Test setting and getting a key-value pair.
    func testDocSetAndGet() async throws {
        let doc = try await node.createDoc()

        let key = "test-key"
        let value = "Hello, Docs!".data(using: .utf8)!

        _ = try await doc.set(author: author, key: key, value: value)

        let entry = try await doc.get(key: key)
        XCTAssertNotNil(entry, "Entry should exist after set")

        let content = try await entry!.content(from: doc)
        XCTAssertEqual(content, value, "Content should match what was set")
    }

    /// Test getting multiple entries with a prefix.
    func testDocGetMany() async throws {
        let doc = try await node.createDoc()

        // Write multiple entries with same prefix
        _ = try await doc.set(author: author, key: "prefix/a", value: "value-a".data(using: .utf8)!)
        _ = try await doc.set(author: author, key: "prefix/b", value: "value-b".data(using: .utf8)!)
        _ = try await doc.set(author: author, key: "other/c", value: "value-c".data(using: .utf8)!)

        // Query entries with prefix and collect into array
        let stream = try await doc.getMany(prefix: "prefix/")
        var entries: [DocEntry] = []
        for try await entry in stream {
            entries.append(entry)
        }

        XCTAssertEqual(entries.count, 2, "Should find 2 entries with prefix 'prefix/'")

        let keys = entries.map { String(data: $0.key, encoding: .utf8)! }
        XCTAssertTrue(keys.contains("prefix/a"))
        XCTAssertTrue(keys.contains("prefix/b"))
    }

    /// Test deleting an entry.
    func testDocDelete() async throws {
        let doc = try await node.createDoc()

        let key = "to-delete"
        _ = try await doc.set(author: author, key: key, value: "temporary".data(using: .utf8)!)

        // Verify it exists
        var entry = try await doc.get(key: key)
        XCTAssertNotNil(entry, "Entry should exist before delete")

        // Delete it
        let count = try await doc.delete(author: author, key: key)
        XCTAssertEqual(count, 1, "Should delete 1 entry")

        // Verify it's gone
        entry = try await doc.get(key: key)
        XCTAssertNil(entry, "Entry should be nil after delete")
    }

    /// Test getting a share ticket.
    func testDocShare() async throws {
        let doc = try await node.createDoc()

        let readTicket = try await doc.shareTicket(mode: .read)
        XCTAssertTrue(readTicket.hasPrefix("doc"), "Read ticket should start with 'doc'")

        let writeTicket = try await doc.shareTicket(mode: .write)
        XCTAssertTrue(writeTicket.hasPrefix("doc"), "Write ticket should start with 'doc'")

        // Write ticket should be different (includes capability)
        XCTAssertNotEqual(readTicket, writeTicket, "Read and write tickets should differ")
    }
}
