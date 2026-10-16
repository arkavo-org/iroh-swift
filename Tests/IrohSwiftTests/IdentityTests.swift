import CryptoKit
import Foundation
import Testing
@testable import IrohSwift

/// Endpoint identity (endpointID / sign) and BLAKE3.
struct IdentityTests {
    private func makeNode(servesBlobs: Bool = true) async throws -> (IrohNode, URL) {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let node = try await IrohNode(
            config: IrohConfig(storagePath: dir, relayEnabled: false, servesBlobs: servesBlobs)
        )
        return (node, dir)
    }

    @Test("endpointID is 64 lowercase hex and equals info().nodeId")
    func endpointIDFormat() async throws {
        let (node, dir) = try await makeNode()
        defer { try? FileManager.default.removeItem(at: dir) }

        let id = try await node.endpointID()
        #expect(id.count == 64)
        #expect(id.allSatisfy { ("0"..."9").contains($0) || ("a"..."f").contains($0) })
        #expect(id == (try await node.info()).nodeId)
        // Stable for the lifetime of the node.
        #expect(id == (try await node.endpointID()))

        try await node.close()
    }

    @Test("sign produces a 64-byte signature CryptoKit verifies against endpointID")
    func signVerifiesWithCryptoKit() async throws {
        let (node, dir) = try await makeNode(servesBlobs: false)
        defer { try? FileManager.default.removeItem(at: dir) }

        let endpointID = try await node.endpointID()
        let bearerToken = "eyJhbGciOiJFUzI1NiJ9.example.token"
        let timestamp = 1_760_000_000
        // The tdf-iroh-s3 publish-session message.
        let message = Data(
            """
            tdf-iroh-s3 publish-session v1
            \(endpointID)
            \(IrohBLAKE3.hex(Data(bearerToken.utf8)))
            \(timestamp)
            """.utf8
        )
        #expect(!message.contains(UInt8(ascii: "\r")))

        let signature = try await node.sign(message)
        #expect(signature.count == 64)

        let publicKey = try Curve25519.Signing.PublicKey(
            rawRepresentation: try #require(Data(hex: endpointID))
        )
        #expect(publicKey.isValidSignature(signature, for: message))

        var tampered = message
        tampered[tampered.count - 1] ^= 0x01
        #expect(!publicKey.isValidSignature(signature, for: tampered))

        // Another node's key does not verify it.
        let (other, otherDir) = try await makeNode()
        defer { try? FileManager.default.removeItem(at: otherDir) }
        let otherKey = try Curve25519.Signing.PublicKey(
            rawRepresentation: try #require(Data(hex: try await other.endpointID()))
        )
        #expect(!otherKey.isValidSignature(signature, for: message))

        try await other.close()
        try await node.close()
    }

    @Test("sign accepts an empty message")
    func signEmptyMessage() async throws {
        let (node, dir) = try await makeNode()
        defer { try? FileManager.default.removeItem(at: dir) }

        let signature = try await node.sign(Data())
        let publicKey = try Curve25519.Signing.PublicKey(
            rawRepresentation: try #require(Data(hex: try await node.endpointID()))
        )
        #expect(publicKey.isValidSignature(signature, for: Data()))

        try await node.close()
    }

    @Test("endpointID and sign throw nodeClosed after close")
    func closedNode() async throws {
        let (node, dir) = try await makeNode()
        defer { try? FileManager.default.removeItem(at: dir) }
        try await node.close()

        let idError = await #expect(throws: IrohError.self) { try await node.endpointID() }
        #expect(idError?.isNodeClosed == true)
        let signError = await #expect(throws: IrohError.self) { try await node.sign(Data("x".utf8)) }
        #expect(signError?.isNodeClosed == true)
    }

    /// Official BLAKE3 test vectors (BLAKE3-team/BLAKE3 test_vectors.json, whose
    /// input is the bytes `i % 251`) plus "abc"; cross-checked with b3sum.
    @Test("IrohBLAKE3.hex matches the official BLAKE3 test vectors")
    func blake3Vectors() {
        func pattern(_ count: Int) -> Data { Data((0..<count).map { UInt8($0 % 251) }) }

        #expect(IrohBLAKE3.hex(Data()) == "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262")
        #expect(IrohBLAKE3.hex(pattern(1)) == "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213")
        #expect(IrohBLAKE3.hex(pattern(1024)) == "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7")
        #expect(IrohBLAKE3.hex(pattern(1025)) == "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444")
        #expect(IrohBLAKE3.hex(Data("abc".utf8)) == "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85")
    }
}

extension Data {
    /// Decode an even-length hex string; nil if it is not valid hex.
    init?(hex: String) {
        guard hex.count.isMultiple(of: 2) else { return nil }
        var bytes = [UInt8]()
        bytes.reserveCapacity(hex.count / 2)
        var index = hex.startIndex
        while index < hex.endIndex {
            let next = hex.index(index, offsetBy: 2)
            guard let byte = UInt8(hex[index..<next], radix: 16) else { return nil }
            bytes.append(byte)
            index = next
        }
        self.init(bytes)
    }
}
