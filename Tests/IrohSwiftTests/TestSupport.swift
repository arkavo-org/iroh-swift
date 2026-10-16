@testable import IrohSwift

extension IrohError {
    /// Case checks for `#expect(throws:)` results in tests.
    var isNodeClosed: Bool { if case .nodeClosed = self { true } else { false } }
    var isNotFound: Bool { if case .notFound = self { true } else { false } }
    var isFetchFailed: Bool { if case .fetchFailed = self { true } else { false } }
    var isPushFailed: Bool { if case .pushFailed = self { true } else { false } }
}
