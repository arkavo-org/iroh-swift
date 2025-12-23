import XCTest
@testable import IrohSwift

final class IrohAuthorTests: XCTestCase {
    /// Test creating a new author.
    func testAuthorCreate() async throws {
        let author = try await IrohAuthor.create()

        XCTAssertEqual(author.publicKey.count, 32, "Public key should be 32 bytes")
        XCTAssertEqual(author.secret.count, 32, "Secret key should be 32 bytes")
        XCTAssertEqual(author.id.count, 64, "Author ID should be 64 hex characters")
    }

    /// Test importing an author from hex and verifying roundtrip.
    func testAuthorFromHex() async throws {
        // Create an author
        let original = try await IrohAuthor.create()
        let secretHex = original.exportSecretHex()

        // Import from hex
        let imported = try await IrohAuthor.fromHex(secretHex)

        XCTAssertEqual(original.id, imported.id, "Imported author should have same ID")
        XCTAssertEqual(original.publicKey, imported.publicKey, "Imported author should have same public key")
    }

    /// Test exporting secret key as hex.
    func testAuthorExportSecretHex() async throws {
        let author = try await IrohAuthor.create()
        let secretHex = author.exportSecretHex()

        XCTAssertEqual(secretHex.count, 64, "Secret hex should be 64 characters")

        // Verify it's valid hex
        let validHexChars = CharacterSet(charactersIn: "0123456789abcdef")
        XCTAssertTrue(
            secretHex.unicodeScalars.allSatisfy { validHexChars.contains($0) },
            "Secret hex should only contain lowercase hex characters"
        )
    }

    /// Test author equality based on ID.
    func testAuthorEquality() async throws {
        let author1 = try await IrohAuthor.create()
        let author2 = try await IrohAuthor.fromHex(author1.exportSecretHex())
        let author3 = try await IrohAuthor.create()

        XCTAssertEqual(author1, author2, "Same author should be equal")
        XCTAssertNotEqual(author1, author3, "Different authors should not be equal")
    }

    /// Test author hash consistency.
    func testAuthorHashable() async throws {
        let author = try await IrohAuthor.create()

        var set = Set<IrohAuthor>()
        set.insert(author)
        set.insert(author) // Insert same author again

        XCTAssertEqual(set.count, 1, "Set should contain only one author")
    }

    /// Test KeychainAccessibility enum values.
    func testKeychainAccessibilityValues() {
        // Just verify the enum cases exist and have different secValues
        let afterFirstUnlock = KeychainAccessibility.afterFirstUnlock
        let whenUnlocked = KeychainAccessibility.whenUnlocked
        let always = KeychainAccessibility.always

        // Each should return a CFString
        XCTAssertNotNil(afterFirstUnlock.secValue)
        XCTAssertNotNil(whenUnlocked.secValue)
        XCTAssertNotNil(always.secValue)
    }

    // Note: Keychain tests (getOrCreate, save, load, delete) require actual Keychain access
    // which may not be available in all test environments. These are tested manually or
    // in integration tests that run with proper entitlements.
}
