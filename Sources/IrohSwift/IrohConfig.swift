import Foundation

/// Configuration for creating an IrohNode.
public struct IrohConfig: Sendable {
    /// Path to the blob store directory.
    public var storagePath: URL

    /// Whether to use n0's public relay servers.
    /// Default: true
    public var relayEnabled: Bool

    /// Create a new IrohConfig with the specified options.
    ///
    /// - Parameters:
    ///   - storagePath: Path to the blob store directory. If nil, defaults to
    ///                  Application Support/iroh (excluded from iCloud backup).
    ///   - relayEnabled: Whether to use n0's public relay servers. Default: true.
    public init(
        storagePath: URL? = nil,
        relayEnabled: Bool = true
    ) {
        self.storagePath = storagePath ?? Self.defaultStoragePath()
        self.relayEnabled = relayEnabled
    }

    /// Default storage path in Application Support, excluded from iCloud backup.
    private static func defaultStoragePath() -> URL {
        let appSupport = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        )[0]
        let irohDir = appSupport.appendingPathComponent("iroh", isDirectory: true)

        // Create directory if needed
        try? FileManager.default.createDirectory(
            at: irohDir,
            withIntermediateDirectories: true
        )

        // Exclude from iCloud backup
        var mutableUrl = irohDir
        var resourceValues = URLResourceValues()
        resourceValues.isExcludedFromBackup = true
        try? mutableUrl.setResourceValues(resourceValues)

        return irohDir
    }
}
