#if canImport(SwiftUI) && canImport(Observation)
import Foundation
import Observation

/// Observable manager for IrohNode, suitable for SwiftUI integration.
///
/// Use this class in SwiftUI views to manage node lifecycle:
/// ```swift
/// struct ContentView: View {
///     @State private var manager = IrohNodeManager()
///
///     var body: some View {
///         Group {
///             if manager.isInitializing {
///                 ProgressView("Starting Iroh...")
///             } else if let node = manager.node {
///                 NodeView(node: node)
///             } else if let error = manager.error {
///                 ErrorView(error: error)
///             }
///         }
///         .task {
///             await manager.initialize()
///         }
///     }
/// }
/// ```
@Observable
public final class IrohNodeManager: Sendable {
    /// The initialized Iroh node, if available.
    @MainActor
    public private(set) var node: IrohNode?

    /// Whether the node is currently being initialized.
    @MainActor
    public private(set) var isInitializing: Bool = false

    /// Any error that occurred during initialization.
    @MainActor
    public private(set) var error: (any Error)?

    public init() {}

    /// Initialize the Iroh node with the given configuration.
    ///
    /// This method is safe to call multiple times; subsequent calls
    /// while initializing or after success are ignored.
    ///
    /// - Parameter config: Configuration options. Uses defaults if not specified.
    @MainActor
    public func initialize(config: IrohConfig = IrohConfig()) async {
        // Prevent multiple initializations
        guard node == nil, !isInitializing else { return }

        isInitializing = true
        error = nil

        do {
            node = try await IrohNode(config: config)
        } catch {
            self.error = error
        }

        isInitializing = false
    }

    /// Reset the manager, destroying any existing node.
    @MainActor
    public func reset() {
        node = nil
        error = nil
        isInitializing = false
    }
}
#endif
