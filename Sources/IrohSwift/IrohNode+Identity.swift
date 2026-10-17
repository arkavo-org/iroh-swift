import Foundation
import IrohSwiftFFI

extension IrohNode {
    /// This node's endpoint id: its Ed25519 public key as 64 lowercase hex characters.
    ///
    /// The same string as `info().nodeId`, and what other nodes pass as `nodeId`
    /// to reach this one. The 32 key bytes are the hex-decoded string, e.g. for
    /// `Curve25519.Signing.PublicKey(rawRepresentation:)`.
    ///
    /// The key is generated when the node is created and is not persisted, so
    /// the id changes for every `IrohNode`. Anything bound to it (such as a
    /// publish session opened with a `sign(_:)` signature) must be used from the
    /// same node instance.
    ///
    /// - Returns: The endpoint id (64 lowercase hex characters).
    /// - Throws: `IrohError.nodeClosed` if the node is closed.
    public func endpointID() async throws -> String {
        try ensureNotClosed()
        guard let pointer = iroh_node_endpoint_id(handle.pointer) else {
            throw IrohError.nodeClosed
        }
        return takeFFIString(pointer)
    }

    /// Sign `message` with this node's endpoint secret key (Ed25519).
    ///
    /// The signature verifies against the public key in `endpointID()`, including
    /// under strict verification (`ed25519-dalek` `verify_strict`, as used by
    /// tdf-iroh-s3), and with CryptoKit's `Curve25519.Signing.PublicKey`.
    ///
    /// - Parameter message: The bytes to sign.
    /// - Returns: The 64-byte Ed25519 signature.
    /// - Throws: `IrohError.nodeClosed` if the node is closed.
    public func sign(_ message: Data) async throws -> Data {
        try ensureNotClosed()
        var signature = Data(count: 64)
        let signed = message.withUnsafeBytes { messageBuffer in
            signature.withUnsafeMutableBytes { signatureBuffer in
                iroh_node_sign(
                    handle.pointer,
                    IrohBytes(
                        data: messageBuffer.baseAddress?.assumingMemoryBound(to: UInt8.self),
                        len: UInt(messageBuffer.count)
                    ),
                    signatureBuffer.baseAddress?.assumingMemoryBound(to: UInt8.self)
                )
            }
        }
        guard signed else { throw IrohError.nodeClosed }
        return signature
    }
}

/// BLAKE3 hashing (the hash iroh uses to address blobs).
public enum IrohBLAKE3 {
    /// BLAKE3 hash of `data` as 64 lowercase hex characters.
    ///
    /// For a raw blob this equals its iroh blob hash.
    public static func hex(_ data: Data) -> String {
        data.withUnsafeBytes { buffer in
            let pointer = iroh_blake3_hex(
                IrohBytes(
                    data: buffer.baseAddress?.assumingMemoryBound(to: UInt8.self),
                    len: UInt(buffer.count)
                )
            )
            return takeFFIString(pointer)
        }
    }
}
