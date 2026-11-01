import Foundation
import IrohSwiftFFI
import Security

/// Keychain accessibility level for author secrets.
///
/// Controls when the keychain item can be accessed. More restrictive levels
/// provide better security but may limit when the author can be used.
public enum KeychainAccessibility: Sendable {
    /// Available after first unlock (default, recommended).
    /// The data is accessible after the device has been unlocked once since boot.
    case afterFirstUnlock

    /// Available only when unlocked.
    /// The data is only accessible when the device is unlocked.
    case whenUnlocked

    var secValue: CFString {
        switch self {
        case .afterFirstUnlock: return kSecAttrAccessibleAfterFirstUnlock
        case .whenUnlocked: return kSecAttrAccessibleWhenUnlocked
        }
    }
}

/// An author identity for signing document entries.
///
/// Authors are cryptographic keypairs used to sign entries in Iroh documents.
/// The secret key should be stored securely in the iOS Keychain.
///
/// Example usage:
/// ```swift
/// // Create or load an author from Keychain
/// let author = try await IrohAuthor.getOrCreate(identifier: "default")
///
/// // Use the author to write to a document
/// try await doc.set(author: author, key: "my-key", value: myData)
///
/// // Export for backup (handle with care!)
/// let secretHex = author.exportSecretHex()
/// ```
public struct IrohAuthor: Sendable, Hashable {
    /// The public author ID as a 64-character hex string.
    public let id: String

    /// The raw public key bytes (32 bytes).
    public let publicKey: Data

    /// The raw secret key bytes (32 bytes).
    /// Kept internal for signing operations.
    internal let secret: Data

    // MARK: - Keychain Constants

    private static let keychainService = "com.iroh.author"

    // MARK: - Initialization

    /// Create an author from raw key data.
    ///
    /// - Parameters:
    ///   - secret: The 32-byte secret key.
    ///   - id: The 32-byte public key ID.
    init(secret: Data, publicKey: Data) {
        precondition(secret.count == 32, "Author secret must be 32 bytes")
        precondition(publicKey.count == 32, "Author public key must be 32 bytes")
        self.secret = secret
        self.publicKey = publicKey
        self.id = publicKey.map { String(format: "%02x", $0) }.joined()
    }

    // MARK: - Factory Methods

    /// Get an existing author from Keychain or create a new one.
    ///
    /// This is the primary way to obtain an author for document operations.
    /// The secret key is stored in the iOS Keychain under the specified identifier.
    ///
    /// - Parameters:
    ///   - identifier: The Keychain identifier for this author. Default is "default".
    ///   - accessibility: The Keychain accessibility level. Default is `.afterFirstUnlock`.
    /// - Returns: The author loaded from or saved to Keychain.
    /// - Throws: `IrohError.authorCreationFailed` if creation fails,
    ///           `IrohError.keychainError` if Keychain operations fail. A new
    ///           author is created only when no item exists: if the stored item
    ///           cannot be read (e.g. the device is locked) or is not a 32-byte
    ///           secret, this throws and leaves the item in place.
    public static func getOrCreate(
        identifier: String = "default",
        accessibility: KeychainAccessibility = .afterFirstUnlock
    ) async throws -> IrohAuthor {
        // Try to load from Keychain first
        // Only "not found" creates an author; any other read failure (a locked
        // device, a missing entitlement) throws instead of replacing the identity.
        if let secretData = try loadFromKeychain(account: identifier) {
            return try fromKeychainSecret(secretData)
        }

        // Create a new author
        let author = try await create()

        // Save to Keychain
        try saveToKeychain(account: identifier, data: author.secret, accessibility: accessibility)

        return author
    }

    /// Create a new random author without storing in Keychain.
    ///
    /// Use this when you want to manage key storage yourself.
    ///
    /// - Returns: A newly created author.
    /// - Throws: `IrohError.authorCreationFailed` if creation fails.
    public static func create() async throws -> IrohAuthor {
        try await withCheckedThrowingContinuation { continuation in
            let box = Unmanaged.passRetained(
                AuthorContinuationBox(continuation)
            ).toOpaque()

            let callback = IrohAuthorCreateCallback(
                userdata: box,
                on_success: { userdata, secret, id in
                    let box = Unmanaged<AuthorContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()

                    let secretData = withUnsafeBytes(of: secret.bytes) { Data($0) }
                    let publicKeyData = withUnsafeBytes(of: id.bytes) { Data($0) }

                    let author = IrohAuthor(secret: secretData, publicKey: publicKeyData)
                    box.continuation.resume(returning: author)
                },
                on_failure: { userdata, errorPtr in
                    let box = Unmanaged<AuthorContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    let message = String(cString: errorPtr!)
                    iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                    box.continuation.resume(throwing: IrohError.authorCreationFailed(message))
                }
            )

            iroh_author_create(callback)
        }
    }

    /// Import an author from a hex-encoded secret key.
    ///
    /// Useful for debugging or restoring from backup.
    ///
    /// - Parameters:
    ///   - secretHex: The 64-character hex-encoded secret key.
    ///   - saveTo: Optional Keychain identifier to save to.
    ///   - accessibility: The Keychain accessibility level if saving. Default is `.afterFirstUnlock`.
    /// - Returns: The imported author.
    /// - Throws: `IrohError.authorCreationFailed` if the hex is invalid.
    public static func fromHex(
        _ secretHex: String,
        saveTo identifier: String? = nil,
        accessibility: KeychainAccessibility = .afterFirstUnlock
    ) async throws -> IrohAuthor {
        let author: IrohAuthor = try await withCheckedThrowingContinuation { continuation in
            let box = Unmanaged.passRetained(
                AuthorContinuationBox(continuation)
            ).toOpaque()

            let callback = IrohAuthorCreateCallback(
                userdata: box,
                on_success: { userdata, secret, id in
                    let box = Unmanaged<AuthorContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()

                    let secretData = withUnsafeBytes(of: secret.bytes) { Data($0) }
                    let publicKeyData = withUnsafeBytes(of: id.bytes) { Data($0) }

                    let author = IrohAuthor(secret: secretData, publicKey: publicKeyData)
                    box.continuation.resume(returning: author)
                },
                on_failure: { userdata, errorPtr in
                    let box = Unmanaged<AuthorContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    let message = String(cString: errorPtr!)
                    iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                    box.continuation.resume(throwing: IrohError.authorCreationFailed(message))
                }
            )

            secretHex.withCString { hexPtr in
                iroh_author_from_hex(hexPtr, callback)
            }
        }

        // Optionally save to Keychain
        if let identifier = identifier {
            try saveToKeychain(account: identifier, data: author.secret, accessibility: accessibility)
        }

        return author
    }

    // MARK: - Export

    /// Export the secret key as a hex string.
    ///
    /// **Warning**: Handle this with extreme care! The secret key allows
    /// signing as this author.
    ///
    /// - Returns: The 64-character hex-encoded secret key.
    public func exportSecretHex() -> String {
        let hexPtr = iroh_author_secret_to_hex(ffiSecret)!
        let hex = String(cString: hexPtr)
        iroh_string_free(hexPtr)
        return hex
    }

    // MARK: - Keychain Management

    /// Delete an author from Keychain.
    ///
    /// - Parameter identifier: The Keychain identifier to delete.
    /// - Throws: `IrohError.keychainError` if deletion fails.
    public static func delete(identifier: String) throws {
        try deleteFromKeychain(account: identifier)
    }

    /// Check if an author exists in Keychain.
    ///
    /// - Parameter identifier: The Keychain identifier to check.
    /// - Returns: True if the author exists.
    public static func exists(identifier: String) -> Bool {
        (try? loadFromKeychain(account: identifier)) != nil
    }

    // MARK: - Internal FFI Helpers

    /// Convert to FFI secret type for document operations.
    internal var ffiSecret: IrohAuthorSecret {
        Self.ffiSecret(from: secret)
    }

    /// Copy a 32-byte secret into the FFI type. Callers check the length.
    private static func ffiSecret(from secretData: Data) -> IrohAuthorSecret {
        var result = IrohAuthorSecret()
        secretData.withUnsafeBytes { buffer in
            withUnsafeMutableBytes(of: &result.bytes) { destBuffer in
                destBuffer.copyMemory(from: buffer)
            }
        }
        return result
    }

    /// Rebuild an author from a secret loaded from the Keychain.
    ///
    /// Throws `IrohError.keychainError` if the stored secret is not 32 bytes,
    /// rather than replacing the stored identity with a new one.
    static func fromKeychainSecret(_ secretData: Data) throws -> IrohAuthor {
        guard secretData.count == 32 else {
            throw IrohError.keychainError("Stored author secret is \(secretData.count) bytes, expected 32")
        }
        let ffiId = iroh_author_id_from_secret(ffiSecret(from: secretData))
        let publicKey = withUnsafeBytes(of: ffiId.bytes) { Data($0) }
        return IrohAuthor(secret: secretData, publicKey: publicKey)
    }

    // MARK: - Private Keychain Helpers

    /// Returns `nil` only if no item exists for `account`.
    private static func loadFromKeychain(account: String) throws -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]

        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        return try keychainData(status: status, result: result)
    }

    /// Interpret a Keychain read: `nil` for `errSecItemNotFound`, the data on
    /// success, and `IrohError.keychainError` for any other status.
    static func keychainData(status: OSStatus, result: AnyObject?) throws -> Data? {
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data else {
            throw IrohError.keychainError("Failed to load author: \(status)")
        }
        return data
    }

    private static func saveToKeychain(
        account: String,
        data: Data,
        accessibility: KeychainAccessibility = .afterFirstUnlock
    ) throws {
        // First try to delete any existing item
        let deleteQuery: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: account
        ]
        SecItemDelete(deleteQuery as CFDictionary)

        // Add new item
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: account,
            kSecValueData as String: data,
            kSecAttrAccessible as String: accessibility.secValue
        ]

        let status = SecItemAdd(query as CFDictionary, nil)

        guard status == errSecSuccess else {
            throw IrohError.keychainError("Failed to save author: \(status)")
        }
    }

    private static func deleteFromKeychain(account: String) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: account
        ]

        let status = SecItemDelete(query as CFDictionary)

        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw IrohError.keychainError("Failed to delete author: \(status)")
        }
    }

    // MARK: - Hashable

    public func hash(into hasher: inout Hasher) {
        hasher.combine(id)
    }

    public static func == (lhs: IrohAuthor, rhs: IrohAuthor) -> Bool {
        lhs.id == rhs.id
    }
}

// MARK: - Continuation Box

private final class AuthorContinuationBox: @unchecked Sendable {
    let continuation: CheckedContinuation<IrohAuthor, Error>

    init(_ continuation: CheckedContinuation<IrohAuthor, Error>) {
        self.continuation = continuation
    }
}
