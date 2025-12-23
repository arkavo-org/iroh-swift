import Foundation

/// Configuration for creating an IrohNode.
public struct IrohConfig: Sendable {
    /// Path to the blob store directory.
    public var storagePath: URL

    /// Whether to use relay servers.
    /// Default: true
    public var relayEnabled: Bool

    /// Custom relay server URL.
    /// If nil, uses n0's public relay servers when relayEnabled is true.
    /// Example: "https://relay.example.com"
    public var customRelayUrl: String?

    /// Whether to enable the Docs engine for syncing documents.
    /// Default: false
    public var docsEnabled: Bool

    /// Create a new IrohConfig with the specified options.
    ///
    /// - Parameters:
    ///   - storagePath: Path to the blob store directory. If nil, defaults to
    ///                  Application Support/iroh (excluded from iCloud backup).
    ///   - relayEnabled: Whether to use relay servers. Default: true.
    ///   - customRelayUrl: Custom relay server URL. If nil, uses n0's public relays.
    ///   - docsEnabled: Whether to enable the Docs engine. Default: false.
    public init(
        storagePath: URL? = nil,
        relayEnabled: Bool = true,
        customRelayUrl: String? = nil,
        docsEnabled: Bool = false
    ) {
        self.storagePath = storagePath ?? Self.defaultStoragePath()
        self.relayEnabled = relayEnabled
        self.customRelayUrl = customRelayUrl
        self.docsEnabled = docsEnabled
    }

    /// Validate the configuration before node creation.
    ///
    /// - Throws: `IrohError.invalidConfiguration` if validation fails.
    public func validate() throws {
        // Check storage path is writable
        let fm = FileManager.default

        // Create directory if it doesn't exist
        if !fm.fileExists(atPath: storagePath.path) {
            do {
                try fm.createDirectory(at: storagePath, withIntermediateDirectories: true)
            } catch {
                throw IrohError.invalidConfiguration(
                    "Cannot create storage directory: \(storagePath.path) - \(error.localizedDescription)"
                )
            }
        }

        // Test writability
        let testFile = storagePath.appendingPathComponent(".iroh-write-test")
        do {
            try Data().write(to: testFile)
            try fm.removeItem(at: testFile)
        } catch {
            throw IrohError.invalidConfiguration(
                "Storage path not writable: \(storagePath.path)"
            )
        }

        // Validate custom relay URL format if provided
        if let relayUrl = customRelayUrl {
            guard let url = URL(string: relayUrl),
                  let scheme = url.scheme?.lowercased(),
                  scheme == "https" || scheme == "http" else {
                throw IrohError.invalidConfiguration(
                    "Invalid relay URL: must be a valid HTTP/HTTPS URL"
                )
            }
        }
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
