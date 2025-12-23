import XCTest
@testable import IrohSwift

final class IrohBlobTests: XCTestCase {
    private var tempDir: URL!
    private var node: IrohNode!

    override func setUp() async throws {
        tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("iroh-blob-tests-\(UUID().uuidString)", isDirectory: true)

        let config = IrohConfig(storagePath: tempDir, relayEnabled: false)
        node = try await IrohNode(config: config)
    }

    override func tearDown() async throws {
        try? FileManager.default.removeItem(at: tempDir)
    }

    /// Test tagging (pinning) a blob.
    func testTagBlob() async throws {
        // Put some data
        let data = Data("Test data for pinning".utf8)
        let ticket = try await node.put(data)

        // Get the hash from the ticket
        let ticketInfo = await validateTicket(ticket)
        XCTAssertTrue(ticketInfo.isValid)
        XCTAssertNotNil(ticketInfo.hash)

        // Tag the blob
        try await node.tagBlob(hash: ticketInfo.hash!, name: "pins/test-content")

        // If we get here without error, tagging succeeded
    }

    /// Test creating a ticket for an existing blob.
    func testCreateTicket() async throws {
        // Put some data
        let data = Data("Test data for ticket creation".utf8)
        let originalTicket = try await node.put(data)

        // Get the hash from the original ticket
        let ticketInfo = await validateTicket(originalTicket)
        XCTAssertNotNil(ticketInfo.hash)

        // Create a new ticket for the same hash
        let newTicket = try await node.createTicket(hash: ticketInfo.hash!)

        XCTAssertTrue(newTicket.hasPrefix("blob"), "Ticket should start with 'blob'")
        XCTAssertFalse(newTicket.isEmpty, "Ticket should not be empty")

        // Validate the new ticket
        let newTicketInfo = await validateTicket(newTicket)
        XCTAssertTrue(newTicketInfo.isValid)
        XCTAssertEqual(newTicketInfo.hash, ticketInfo.hash, "Hash should match original")
    }

    /// Test untagging (unpinning) a blob.
    func testUntagBlob() async throws {
        // Put some data
        let data = Data("Test data for unpinning".utf8)
        let ticket = try await node.put(data)

        // Get the hash from the ticket
        let ticketInfo = await validateTicket(ticket)

        // Tag it
        let tagName = "pins/test-unpin"
        try await node.tagBlob(hash: ticketInfo.hash!, name: tagName)

        // Untag it
        try await node.untagBlob(name: tagName)

        // If we get here without error, untagging succeeded
    }

    /// Test BlobFormat enum values.
    func testBlobFormatValues() {
        XCTAssertEqual(BlobFormat.raw.rawValue, 0)
        XCTAssertEqual(BlobFormat.hashSeq.rawValue, 1)
    }

    /// Test tagging with HashSeq format.
    func testTagBlobWithHashSeqFormat() async throws {
        // Put some data
        let data = Data("Test data for HashSeq format".utf8)
        let ticket = try await node.put(data)

        // Get the hash from the ticket
        let ticketInfo = await validateTicket(ticket)

        // Tag with HashSeq format
        try await node.tagBlob(hash: ticketInfo.hash!, name: "pins/hashseq-test", format: .hashSeq)

        // If we get here without error, tagging with format succeeded
    }

    /// Test creating ticket with HashSeq format.
    func testCreateTicketWithHashSeqFormat() async throws {
        // Put some data
        let data = Data("Test data for HashSeq ticket".utf8)
        let ticket = try await node.put(data)

        // Get the hash from the ticket
        let ticketInfo = await validateTicket(ticket)

        // Create ticket with HashSeq format
        let newTicket = try await node.createTicket(hash: ticketInfo.hash!, format: .hashSeq)

        XCTAssertTrue(newTicket.hasPrefix("blob"), "Ticket should start with 'blob'")

        // Validate and check recursive flag
        let newTicketInfo = await validateTicket(newTicket)
        XCTAssertTrue(newTicketInfo.isRecursive, "Ticket should be marked as recursive for HashSeq")
    }
}
