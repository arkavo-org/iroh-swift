import Foundation
import IrohSwiftFFI

extension IrohNode {
    /// Get information about this node.
    ///
    /// - Returns: Node information including ID, relay URL, and connection status.
    /// - Throws: `IrohError.nodeCreationFailed` if the info cannot be retrieved.
    public func info() async throws -> NodeInfo {
        try await withCheckedThrowingContinuation { continuation in
            let box = Unmanaged.passRetained(
                ContinuationBox<NodeInfo>(continuation)
            ).toOpaque()

            let callback = IrohNodeInfoCallback(
                userdata: box,
                on_success: { userdata, info in
                    let box = Unmanaged<ContinuationBox<NodeInfo>>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()

                    let nodeId = String(cString: info.node_id)
                    iroh_string_free(UnsafeMutablePointer(mutating: info.node_id))

                    let relayUrl: String?
                    if info.relay_url != nil {
                        relayUrl = String(cString: info.relay_url)
                        iroh_string_free(UnsafeMutablePointer(mutating: info.relay_url))
                    } else {
                        relayUrl = nil
                    }

                    let nodeInfo = NodeInfo(
                        nodeId: nodeId,
                        relayUrl: relayUrl,
                        isConnected: info.is_connected
                    )
                    box.continuation.resume(returning: nodeInfo)
                },
                on_failure: { userdata, errorPtr in
                    let box = Unmanaged<ContinuationBox<NodeInfo>>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    let message = String(cString: errorPtr!)
                    iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                    box.continuation.resume(throwing: IrohError.nodeCreationFailed(message))
                }
            )

            iroh_node_info(handle.pointer, callback)
        }
    }
}

/// Validate and parse a ticket string without requiring a node.
///
/// - Parameter ticket: The ticket string to validate.
/// - Returns: Information about the ticket.
public func validateTicket(_ ticket: String) async -> TicketInfo {
    await withCheckedContinuation { continuation in
        ticket.withCString { ticketPtr in
            // Use a simple wrapper for the non-throwing continuation
            final class TicketContinuationBox: @unchecked Sendable {
                let continuation: CheckedContinuation<TicketInfo, Never>

                init(_ continuation: CheckedContinuation<TicketInfo, Never>) {
                    self.continuation = continuation
                }
            }

            let box = Unmanaged.passRetained(
                TicketContinuationBox(continuation)
            ).toOpaque()

            let callback = IrohTicketValidateCallback(
                userdata: box,
                on_complete: { userdata, info in
                    let box = Unmanaged<TicketContinuationBox>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()

                    var hash: String?
                    var nodeId: String?

                    if info.is_valid {
                        if info.hash != nil {
                            hash = String(cString: info.hash)
                            iroh_string_free(UnsafeMutablePointer(mutating: info.hash))
                        }
                        if info.node_id != nil {
                            nodeId = String(cString: info.node_id)
                            iroh_string_free(UnsafeMutablePointer(mutating: info.node_id))
                        }
                    }

                    let ticketInfo = TicketInfo(
                        isValid: info.is_valid,
                        hash: hash,
                        nodeId: nodeId,
                        isRecursive: info.is_recursive
                    )
                    box.continuation.resume(returning: ticketInfo)
                }
            )

            iroh_validate_ticket(ticketPtr, callback)
        }
    }
}

// MARK: - Internal Helpers

/// Box for passing Swift continuations through FFI callbacks (for NodeInfo).
/// This needs to be accessible from this file since we use it directly.
private final class ContinuationBox<T>: @unchecked Sendable {
    let continuation: CheckedContinuation<T, Error>

    init(_ continuation: CheckedContinuation<T, Error>) {
        self.continuation = continuation
    }
}
