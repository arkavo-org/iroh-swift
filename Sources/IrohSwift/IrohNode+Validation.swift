import Foundation

extension IrohNode {
    /// Validate that a ticket string has valid format.
    ///
    /// This performs basic format validation without network calls.
    /// A valid ticket starts with "blob" prefix and has sufficient length.
    ///
    /// - Parameter ticket: The ticket string to validate.
    /// - Returns: `true` if the ticket has valid format, `false` otherwise.
    public static func isValidTicket(_ ticket: String) -> Bool {
        // Basic validation: must start with "blob" and be non-trivial length
        // Real tickets are base32-encoded and quite long (100+ chars typically)
        guard ticket.hasPrefix("blob") else {
            return false
        }

        // Minimum length check - a real blob ticket is much longer
        // "blob" + base32-encoded hash + node info
        guard ticket.count >= 10 else {
            return false
        }

        return true
    }
}
