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

    /// Whether this node serves blobs to other nodes (accepts the iroh-blobs protocol).
    ///
    /// When `false` the node is client-only: it can still `fetch(hash:fromNode:...)`,
    /// `get(ticket:)` and `pushToNode(...)`, but peers cannot download blobs from
    /// it — their blobs connections are refused at the handshake. Tickets created
    /// by `put(_:)` on a client-only node are not fetchable by others. With
    /// `docsEnabled`, peers cannot pull this node's document content either.
    /// Whether peers may push blobs to a serving node is `acceptsPushes`.
    /// Default: true
    public var servesBlobs: Bool

    /// Whether peers may push blobs into this node's store.
    ///
    /// Only meaningful when `servesBlobs` is `true`; setting it on a client-only
    /// node fails `validate()` with `IrohError.invalidConfiguration`. When `false`,
    /// a serving node refuses push requests and writes nothing; peers can still
    /// download its blobs. A peer pushing to it is not reliably told: a small
    /// push can report success even though nothing was stored.
    /// Default: false, because accepting pushes lets any peer that can reach the
    /// node write to its disk.
    public var acceptsPushes: Bool

    /// Whether this node resolves endpoint IDs on the BitTorrent Mainline DHT
    /// (pkarr), in addition to the default n0 lookups.
    ///
    /// With it on, `fetch(hash:fromNode:...)` and `pushToNode(...)` can reach a
    /// node by its endpoint ID alone: pass no `relayUrl` and no `directAddrs`,
    /// and the node is looked up on the DHT. Explicit addresses still apply
    /// alongside. This node only reads from the DHT; it publishes nothing
    /// unless `dhtPublish` is also set. The DHT client opens its own UDP socket
    /// and contacts public bootstrap nodes, so it is off by default.
    ///
    /// Creating the node resolves the bootstrap hostnames first (up to about 8
    /// seconds in total, never blocking DNS inside the DHT library). If none
    /// resolves, for example when started offline, the node starts without the
    /// DHT and `info().dhtActive` is `false`; it is not retried, so recreate the
    /// node to get DHT lookup.
    /// Default: false
    public var dhtLookup: Bool

    /// Whether this node also publishes its own address to the DHT, so peers
    /// with `dhtLookup` can find it by endpoint ID.
    ///
    /// Requires `dhtLookup` and `servesBlobs`: a client-only node is not
    /// dialled, so it never publishes, and setting this on one fails
    /// `validate()` with `IrohError.invalidConfiguration`. While relays are
    /// enabled only the relay URL is published, not IP addresses (the DHT is
    /// public). With `relayEnabled: false` the node's direct addresses are
    /// published instead, because they are all a peer could use.
    /// Default: false
    public var dhtPublish: Bool

    /// Create a new IrohConfig with the specified options.
    ///
    /// - Parameters:
    ///   - storagePath: Path to the blob store directory. If nil, defaults to
    ///                  Application Support/iroh (excluded from iCloud backup).
    ///   - relayEnabled: Whether to use relay servers. Default: true.
    ///   - customRelayUrl: Custom relay server URL. If nil, uses n0's public relays.
    ///   - docsEnabled: Whether to enable the Docs engine. Default: false.
    ///   - servesBlobs: Whether to serve blobs to other nodes. `false` makes the
    ///                  node client-only (fetch and push still work). Default: true.
    ///   - acceptsPushes: Whether peers may push blobs into this node's store.
    ///                    Only meaningful when `servesBlobs` is true. Default: false,
    ///                    because accepting pushes lets any peer that can reach the
    ///                    node write to its disk.
    ///   - dhtLookup: Whether to resolve endpoint IDs on the Mainline DHT, so a
    ///                node can be fetched from or pushed to by ID alone.
    ///                Default: false.
    ///   - dhtPublish: Whether to also publish this node's address to the DHT.
    ///                 Requires `dhtLookup` and `servesBlobs`. Default: false.
    public init(
        storagePath: URL? = nil,
        relayEnabled: Bool = true,
        customRelayUrl: String? = nil,
        docsEnabled: Bool = false,
        servesBlobs: Bool = true,
        acceptsPushes: Bool = false,
        dhtLookup: Bool = false,
        dhtPublish: Bool = false
    ) {
        self.storagePath = storagePath ?? Self.defaultStoragePath()
        self.relayEnabled = relayEnabled
        self.customRelayUrl = customRelayUrl
        self.docsEnabled = docsEnabled
        self.servesBlobs = servesBlobs
        self.acceptsPushes = acceptsPushes
        self.dhtLookup = dhtLookup
        self.dhtPublish = dhtPublish
    }

    /// Validate the configuration before node creation.
    ///
    /// - Throws: `IrohError.invalidConfiguration` if validation fails.
    public func validate() throws {
        // Checked first so a rejected config touches nothing on disk.
        if acceptsPushes && !servesBlobs {
            throw IrohError.invalidConfiguration(
                "acceptsPushes requires servesBlobs: a client-only node cannot receive pushes"
            )
        }

        if dhtPublish && !dhtLookup {
            throw IrohError.invalidConfiguration("dhtPublish requires dhtLookup")
        }
        if dhtPublish && !servesBlobs {
            throw IrohError.invalidConfiguration(
                "dhtPublish requires servesBlobs: a client-only node is not dialled, so it has nothing to publish"
            )
        }

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
