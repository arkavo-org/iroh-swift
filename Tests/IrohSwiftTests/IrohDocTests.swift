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
        if let node = node {
            try? await node.close()
            self.node = nil
        }
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

    /// getMany delivers every match, not only the newest 100.
    func testDocGetManyReturnsMoreThan100Entries() async throws {
        let doc = try await node.createDoc()
        for i in 0..<150 {
            _ = try await doc.set(author: author, key: "bulk/\(i)", value: Data("v\(i)".utf8))
        }
        var count = 0
        for try await _ in try await doc.getMany(prefix: "bulk/") { count += 1 }
        XCTAssertEqual(count, 150)
    }

    /// Two imported authors, ordered so the first sorts first by author ID.
    /// A query sorted by author returns the first author's entry before the second's.
    private func twoAuthorsInIdOrder() async throws -> (IrohAuthor, IrohAuthor) {
        let other = try await IrohAuthor.create()
        try await node.importAuthor(other)
        return author.id < other.id ? (author, other) : (other, author)
    }

    /// When two authors write the same key, get and getMany return the newer write.
    func testDocGetReturnsLatestWriteAcrossAuthors() async throws {
        let doc = try await node.createDoc()
        let (first, second) = try await twoAuthorsInIdOrder()
        _ = try await doc.set(author: first, key: "k", value: Data("old".utf8))
        _ = try await doc.set(author: second, key: "k", value: Data("new".utf8))

        let entry = try await doc.get(key: "k")
        let content = try await XCTUnwrap(entry).content(from: doc)
        XCTAssertEqual(content, Data("new".utf8))

        var entries: [DocEntry] = []
        for try await e in try await doc.getMany(prefix: "k") { entries.append(e) }
        XCTAssertEqual(entries.count, 1, "one entry per key, the latest")
        XCTAssertEqual(entries.first?.authorId, second.id)
    }

    /// A delete by one author hides the key, though another author's older entry remains.
    func testDocGetAfterDeleteByAnotherAuthorReturnsNil() async throws {
        let doc = try await node.createDoc()
        let (first, second) = try await twoAuthorsInIdOrder()
        _ = try await doc.set(author: first, key: "k", value: Data("old".utf8))
        _ = try await doc.delete(author: second, key: "k")

        let entry = try await doc.get(key: "k")
        XCTAssertNil(entry)
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

    /// delete is a prefix delete: it also removes the author's longer keys.
    func testDocDeleteRemovesKeysWithThePrefix() async throws {
        let doc = try await node.createDoc()
        for key in ["user", "user1", "user/settings", "other"] {
            _ = try await doc.set(author: author, key: key, value: Data(key.utf8))
        }

        let count = try await doc.delete(author: author, key: "user")
        XCTAssertEqual(count, 3)

        var remaining: [String] = []
        for try await entry in try await doc.getMany(prefix: "") {
            remaining.append(try XCTUnwrap(entry.keyString))
        }
        XCTAssertEqual(remaining, ["other"])
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
