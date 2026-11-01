import Foundation
import IrohSwiftFFI
import Testing
@testable import IrohSwift

/// Converting FFI document entries must not trap on malformed input.
@Suite struct DocEntryTests {
    private func entry(
        key: UnsafeMutablePointer<UInt8>? = nil,
        keyLen: UInt = 0,
        contentHash: UnsafeMutablePointer<CChar>? = nil
    ) -> IrohDocEntry {
        IrohDocEntry(
            author_id: IrohAuthorId(),
            key: IrohOwnedBytes(data: key, len: keyLen, capacity: keyLen),
            content_hash: contentHash,
            content_size: 0,
            timestamp: 0
        )
    }

    @Test func nullEntryPointerConvertsToNil() {
        #expect(DocEntry.consume(nil) == nil)
    }

    @Test func entryWithoutContentHashConvertsToNil() {
        #expect(DocEntry(from: entry()) == nil)
    }

    @Test func entryWithNullKeyButNonZeroLengthConvertsToNil() throws {
        let hash = try #require(strdup("abc"))
        defer { free(hash) }
        #expect(DocEntry(from: entry(keyLen: 4, contentHash: hash)) == nil)
    }

    @Test func entryWithEmptyKeyConverts() throws {
        let hash = try #require(strdup("abc"))
        defer { free(hash) }
        let converted = try #require(DocEntry(from: entry(contentHash: hash)))
        #expect(converted.key.isEmpty)
        #expect(converted.contentHash == "abc")
    }

    @Test func malformedEntryInEventIsDropped() {
        var malformed = entry()
        withUnsafePointer(to: &malformed) { ptr in
            let local = IrohDocEvent(event_type: InsertLocal, entry: ptr, peer_id: nil, content_hash: nil)
            #expect(DocEvent.from(local) == nil)
            // An event type that carries no entry ignores a malformed one
            let pending = IrohDocEvent(event_type: PendingContentReady, entry: ptr, peer_id: nil, content_hash: nil)
            #expect(DocEvent.from(pending) != nil)
        }
    }
}
