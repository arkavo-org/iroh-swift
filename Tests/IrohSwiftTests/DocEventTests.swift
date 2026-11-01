import Foundation
import IrohSwiftFFI
import Testing
@testable import IrohSwift

/// Converting FFI document events must not trap on malformed input.
@Suite struct DocEventTests {
    private func event(
        _ type: IrohDocEventType,
        peerId: UnsafePointer<CChar>? = nil,
        contentHash: UnsafePointer<CChar>? = nil
    ) -> IrohDocEvent {
        IrohDocEvent(event_type: type, entry: nil, peer_id: peerId, content_hash: contentHash)
    }

    @Test func unknownEventTypeIsDropped() {
        #expect(DocEvent.from(event(IrohDocEventType(rawValue: 99))) == nil)
    }

    @Test func insertEventWithoutEntryIsDropped() {
        #expect(DocEvent.from(event(InsertLocal)) == nil)
        #expect(DocEvent.from(event(InsertRemote)) == nil)
    }

    @Test func peerEventWithoutPeerIdIsDropped() {
        for type in [NeighborUp, NeighborDown, SyncFinished] {
            #expect(DocEvent.from(event(type)) == nil)
        }
    }

    @Test func contentReadyWithoutHashIsDropped() {
        #expect(DocEvent.from(event(ContentReady)) == nil)
    }

    @Test func wellFormedPeerEventConverts() throws {
        let peer = try #require(strdup("abc"))
        defer { free(peer) }
        let converted = DocEvent.from(event(NeighborUp, peerId: peer))
        guard case .neighborUp(let peerId)? = converted else {
            Issue.record("expected .neighborUp, got \(String(describing: converted))")
            return
        }
        #expect(peerId == "abc")
    }

    @Test func pendingContentReadyNeedsNoPayload() {
        guard case .pendingContentReady? = DocEvent.from(event(PendingContentReady)) else {
            Issue.record("expected .pendingContentReady")
            return
        }
    }
}
