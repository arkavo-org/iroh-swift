@testable import IrohSwift

extension IrohError {
    /// Case checks for `#expect(throws:)` results in tests.
    var isNodeClosed: Bool { if case .nodeClosed = self { true } else { false } }
}
