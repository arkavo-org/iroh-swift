import Foundation
import IrohSwift

@main
struct IrohCLI {
    static func main() async throws {
        let args = CommandLine.arguments
        let command = args.count > 1 ? args[1] : "blob"

        switch command {
        case "blob":
            try await runBlobDemo()
        case "docs":
            try await runDocsDemo()
        case "docs-join":
            guard args.count > 2 else {
                print("Usage: iroh-cli docs-join <ticket>")
                return
            }
            try await runDocsJoinDemo(ticket: args[2])
        default:
            printUsage()
        }
    }

    // MARK: - Blob Demo (existing functionality)

    static func runBlobDemo() async throws {
        print("Starting Iroh node with relay enabled...")

        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("iroh-cli-\(UUID().uuidString)", isDirectory: true)

        let config = IrohConfig(storagePath: tempDir, relayEnabled: true)
        let node = try await IrohNode(config: config)

        print("Node started successfully!")

        // Put test data
        let testData = Data("Hello from iroh-swift CLI! Timestamp: \(Date())".utf8)
        let ticket = try await node.put(testData)

        print("")
        print("=== TICKET ===")
        print(ticket)
        print("==============")
        print("")
        print("Share this ticket with iroh.arkavo.net to download the data.")
        print("Press Enter to exit...")

        // Keep node alive so remote can fetch
        _ = readLine()

        print("Shutting down...")
        try? FileManager.default.removeItem(at: tempDir)
    }

    // MARK: - Docs Demo (create document, write values, share ticket)

    static func runDocsDemo() async throws {
        print("Starting Iroh node with Docs enabled...")

        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("iroh-cli-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let config = IrohConfig(
            storagePath: tempDir,
            relayEnabled: true,
            docsEnabled: true
        )
        let node = try await IrohNode(config: config)

        // Create author (persisted to Keychain as "cli-demo")
        let author = try await IrohAuthor.getOrCreate(identifier: "cli-demo")
        try await node.importAuthor(author)
        print("Author ID: \(author.id.prefix(16))...")

        // Create a new document
        let doc = try await node.createDoc()
        let namespaceId = await doc.namespaceId
        print("Created document: \(namespaceId.prefix(16))...")

        // Write some key-value pairs
        _ = try await doc.set(author: author, key: "greeting",
                              value: "Hello from iroh-swift!".data(using: .utf8)!)
        _ = try await doc.set(author: author, key: "timestamp",
                              value: "\(Date())".data(using: .utf8)!)
        print("Wrote 2 entries to document")

        // Read back
        if let entry = try await doc.get(key: "greeting") {
            let content = try await entry.content(from: doc)
            print("Read back: \(String(data: content, encoding: .utf8) ?? "?")")
        }

        // Get share ticket
        let ticket = try await doc.shareTicket(mode: .write)
        print("")
        print("=== DOC TICKET (write access) ===")
        print(ticket)
        print("=================================")
        print("")
        print("Join from another terminal with:")
        print("  swift run iroh-cli docs-join <ticket>")
        print("")
        print("Press Enter to exit...")

        _ = readLine()
        print("Shutting down...")
    }

    // MARK: - Docs Join Demo (join document, subscribe to events)

    static func runDocsJoinDemo(ticket: String) async throws {
        print("Joining document...")

        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("iroh-cli-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let config = IrohConfig(
            storagePath: tempDir,
            relayEnabled: true,
            docsEnabled: true
        )
        let node = try await IrohNode(config: config)

        let author = try await IrohAuthor.getOrCreate(identifier: "cli-joiner")
        try await node.importAuthor(author)
        let doc = try await node.joinDoc(ticket: ticket)
        let namespaceId = await doc.namespaceId

        print("Joined document: \(namespaceId.prefix(16))...")
        print("Author ID: \(author.id.prefix(16))...")
        print("")

        // Write our arrival announcement
        let arrivalKey = "joined/\(author.id.prefix(8))"
        _ = try await doc.set(author: author, key: arrivalKey,
                              value: "Joined at \(Date())".data(using: .utf8)!)
        print("Announced arrival with key: \(arrivalKey)")
        print("")
        print("Subscribing to events (Ctrl+C to exit)...")
        print("")

        // Subscribe and print events
        let subscription = try await doc.subscribe()
        for try await event in subscription {
            switch event {
            case .insertLocal(let entry):
                let key = String(data: entry.key, encoding: .utf8) ?? "raw"
                print("[LOCAL] Wrote '\(key)'")

            case .insertRemote(let peer, let entry):
                let key = String(data: entry.key, encoding: .utf8) ?? "raw"
                print("[REMOTE from \(peer.prefix(8))] Received '\(key)'")

                // Fetch and print the actual text content for small messages
                if entry.contentSize < 1024 {
                    do {
                        let data = try await entry.content(from: doc)
                        let text = String(data: data, encoding: .utf8) ?? "binary"
                        print("   -> Content: \"\(text)\"")
                    } catch {
                        print("   -> (content pending)")
                    }
                }

            case .neighborUp(let peer):
                print("[PEER UP] \(peer.prefix(8))...")

            case .neighborDown(let peer):
                print("[PEER DOWN] \(peer.prefix(8))...")

            case .syncFinished(let peer):
                print("[SYNC DONE] with \(peer.prefix(8))")

            case .contentReady(let hash):
                print("[CONTENT READY] \(hash.prefix(16))...")

            case .pendingContentReady:
                print("[ALL CONTENT READY]")
            }
        }
    }

    // MARK: - Help

    static func printUsage() {
        print("""
        iroh-cli - Iroh Swift CLI Demo

        Usage: iroh-cli <command>

        Commands:
          blob                  Put blob data and print ticket (default)
          docs                  Create a doc, write values, print share ticket
          docs-join <ticket>    Join a doc and subscribe to events

        Examples:
          swift run iroh-cli blob
          swift run iroh-cli docs
          swift run iroh-cli docs-join docabc123...
        """)
    }
}
