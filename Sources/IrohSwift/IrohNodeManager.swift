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

    /// Current download progress, if a download is in progress.
    @MainActor
    public private(set) var downloadProgress: DownloadProgress?

    /// Whether a download is currently in progress.
    @MainActor
    public var isDownloading: Bool {
        downloadProgress != nil
    }

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

    /// Download data from a ticket with progress tracking.
    ///
    /// Progress updates are automatically delivered on the MainActor,
    /// making this safe to use directly in SwiftUI views.
    ///
    /// Example:
    /// ```swift
    /// struct DownloadView: View {
    ///     @State private var manager = IrohNodeManager()
    ///
    ///     var body: some View {
    ///         VStack {
    ///             if let progress = manager.downloadProgress {
    ///                 ProgressView(value: progress.fraction ?? 0)
    ///             }
    ///             Button("Download") {
    ///                 Task {
    ///                     let data = try await manager.download(ticket: someTicket)
    ///                 }
    ///             }
    ///             .disabled(manager.isDownloading)
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// - Parameter ticket: The ticket string to download from.
    /// - Returns: The downloaded data.
    /// - Throws: `IrohError.nodeCreationFailed` if node is not initialized,
    ///           `IrohError.getFailed` if download fails.
    @MainActor
    public func download(ticket: String) async throws -> Data {
        guard let node = node else {
            throw IrohError.nodeCreationFailed("Node not initialized")
        }

        downloadProgress = DownloadProgress(downloaded: 0, total: 0)

        do {
            let data = try await node.get(ticket: ticket) { [weak self] progress in
                Task { @MainActor in
                    self?.downloadProgress = progress
                }
            }
            downloadProgress = nil
            return data
        } catch {
            downloadProgress = nil
            throw error
        }
    }

    /// Download data from a ticket with options and progress tracking.
    ///
    /// - Parameters:
    ///   - ticket: The ticket string to download from.
    ///   - options: Operation options including timeout.
    /// - Returns: The downloaded data.
    /// - Throws: `IrohError.nodeCreationFailed` if node is not initialized,
    ///           `IrohError.timeout` if the operation times out,
    ///           `IrohError.getFailed` if download fails.
    @MainActor
    public func download(ticket: String, options: OperationOptions) async throws -> Data {
        guard let node = node else {
            throw IrohError.nodeCreationFailed("Node not initialized")
        }

        downloadProgress = DownloadProgress(downloaded: 0, total: 0)

        do {
            let data = try await node.get(ticket: ticket, options: options)
            downloadProgress = nil
            return data
        } catch {
            downloadProgress = nil
            throw error
        }
    }

    /// Reset the manager, destroying any existing node.
    @MainActor
    public func reset() {
        node = nil
        error = nil
        isInitializing = false
        downloadProgress = nil
    }
}
#endif
