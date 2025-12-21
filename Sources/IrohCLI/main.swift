import Foundation
import IrohSwift

@main
struct IrohCLI {
    static func main() async throws {
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
        print("Keeping node alive for 60 seconds...")

        // Keep node alive so remote can fetch
        try await Task.sleep(nanoseconds: 60_000_000_000)

        print("Done. Cleaning up...")
        try? FileManager.default.removeItem(at: tempDir)
    }
}
