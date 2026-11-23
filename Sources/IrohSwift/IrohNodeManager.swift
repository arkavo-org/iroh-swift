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
        activeDownloadCount > 0 || downloadProgress != nil
    }

    /// Process-wide pending shutdown tasks keyed by normalized `storagePath`.
    /// Serializes store reopening even when a consumer discards an old
    /// `IrohNodeManager` after calling `reset()` and creates a new instance
    /// targeting the same `FsStore` directory.
    @MainActor
    private static var pendingCleanups: [String: (id: UInt64, task: Task<Void, Never>)] = [:]

    @MainActor
    private static var nextCleanupId: UInt64 = 0

    @MainActor
    @ObservationIgnored
    private var activeDownloadCount: Int = 0

    @MainActor
    @ObservationIgnored
    private var downloadGeneration: UInt64 = 0

    @MainActor
    @ObservationIgnored
    private var initGeneration: UInt64 = 0

    @MainActor
    @ObservationIgnored
    private var activeStoragePath: String?

    @MainActor
    @ObservationIgnored
    private var initTask: Task<IrohNode, Error>?

    @MainActor
    @ObservationIgnored
    private var cleanupTask: Task<Void, Never>?

    public init() {}

    /// Initialize the Iroh node with the given configuration.
    ///
    /// This method is safe to call multiple times; subsequent calls
    /// while initializing or after success are ignored. If `reset()` was
    /// called recently (on this instance or a prior instance targeting the
    /// same `storagePath`), initialization waits for the prior node to finish
    /// closing so `FsStore` does not hit a redb lock conflict.
    ///
    /// - Parameter config: Configuration options. Uses defaults if not specified.
    @MainActor
    public func initialize(config: IrohConfig = IrohConfig()) async {
        // Prevent multiple initializations
        guard node == nil, !isInitializing else { return }

        isInitializing = true
        error = nil

        let normalizedPath = config.storagePath.standardizedFileURL.path
        activeStoragePath = normalizedPath
        let generation = initGeneration
        let priorCleanup = cleanupTask
        let sharedCleanup = Self.pendingCleanups[normalizedPath]?.task
        let task = Task<IrohNode, Error> {
            await priorCleanup?.value
            await sharedCleanup?.value
            try Task.checkCancellation()
            return try await IrohNode(config: config)
        }
        initTask = task

        do {
            let created = try await task.value
            guard initGeneration == generation else {
                // `reset()` was called while initializing; ensure the node is closed.
                try? await created.close()
                return
            }
            node = created
            initTask = nil
            isInitializing = false
        } catch {
            guard initGeneration == generation else { return }
            self.error = error
            initTask = nil
            isInitializing = false
        }
    }

    /// Download data from a ticket with progress tracking.
    ///
    /// Progress updates are delivered strictly in order on the MainActor and
    /// cancelled/drained before returning so `isDownloading` never sticks `true`.
    ///
    /// - Parameter ticket: The ticket string to download from.
    /// - Returns: The downloaded data.
    /// - Throws: `IrohError.nodeCreationFailed` if node is not initialized,
    ///           `IrohError.getFailed` if download fails,
    ///           `CancellationError` if cancelled.
    @MainActor
    public func download(ticket: String) async throws -> Data {
        try await download(ticket: ticket, options: .default)
    }

    /// Download data from a ticket with options and progress tracking.
    ///
    /// - Parameters:
    ///   - ticket: The ticket string to download from.
    ///   - options: Operation options including timeout and maximum blob size.
    /// - Returns: The downloaded data.
    /// - Throws: `IrohError.nodeCreationFailed` if node is not initialized,
    ///           `IrohError.timeout` if the operation times out,
    ///           `IrohError.sizeLimitExceeded` if the blob exceeds `options.maxSize`,
    ///           `IrohError.getFailed` if download fails,
    ///           `CancellationError` if cancelled.
    @MainActor
    public func download(ticket: String, options: OperationOptions) async throws -> Data {
        guard let node = node else {
            throw IrohError.nodeCreationFailed("Node not initialized")
        }

        activeDownloadCount += 1
        downloadGeneration &+= 1
        let generation = downloadGeneration
        downloadProgress = DownloadProgress(downloaded: 0, total: 0)

        let (stream, continuation) = AsyncStream.makeStream(of: DownloadProgress.self)
        let progressTask = Task { @MainActor [weak self] in
            for await progress in stream {
                guard let self, self.downloadGeneration == generation else { break }
                self.downloadProgress = progress
            }
        }

        defer {
            continuation.finish()
            progressTask.cancel()
            activeDownloadCount = max(0, activeDownloadCount - 1)
            if downloadGeneration == generation {
                downloadGeneration &+= 1
                if activeDownloadCount == 0 {
                    downloadProgress = nil
                }
            } else if activeDownloadCount == 0 {
                downloadProgress = nil
            }
        }

        return try await node.get(ticket: ticket, options: options) { progress in
            continuation.yield(progress)
        }
    }

    /// Reset the manager, cancelling any in-flight initialization and closing
    /// any existing node cleanly in the background.
    ///
    /// A subsequent `initialize(config:)` — on this instance or a newly created
    /// `IrohNodeManager` targeting the same `storagePath` — automatically awaits
    /// the prior node's shutdown before reopening the blob store. Use
    /// `resetAndWait()` if you want to explicitly await shutdown completion
    /// before deleting storage directories.
    @MainActor
    public func reset() {
        initGeneration &+= 1
        downloadGeneration &+= 1
        activeDownloadCount = 0

        let previousInit = initTask
        initTask = nil
        previousInit?.cancel()

        let previousNode = node
        let pathToClean = activeStoragePath
        activeStoragePath = nil
        node = nil
        error = nil
        isInitializing = false
        downloadProgress = nil

        let priorCleanup = cleanupTask
        let priorShared = pathToClean.flatMap { Self.pendingCleanups[$0]?.task }
        Self.nextCleanupId &+= 1
        let cleanupId = Self.nextCleanupId

        let task = Task { @MainActor in
            await priorCleanup?.value
            await priorShared?.value
            if let previousInit {
                if let created = try? await previousInit.value {
                    try? await created.close()
                }
            }
            if let previousNode {
                try? await previousNode.close()
            }
            if let pathToClean, Self.pendingCleanups[pathToClean]?.id == cleanupId {
                Self.pendingCleanups.removeValue(forKey: pathToClean)
            }
        }
        cleanupTask = task
        if let pathToClean {
            Self.pendingCleanups[pathToClean] = (id: cleanupId, task: task)
        }
    }

    /// Reset the manager and wait until any in-flight initialization and node
    /// shutdown have completely finished.
    @MainActor
    public func resetAndWait() async {
        reset()
        await cleanupTask?.value
    }
}
#endif
