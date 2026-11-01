import Foundation
import Testing
@testable import IrohSwift

/// Converting FFI document entries must not trap on a null pointer.
@Suite struct DocEntryTests {
    @Test func nullEntryPointerConvertsToNil() {
        #expect(DocEntry.consume(nil) == nil)
    }
}
