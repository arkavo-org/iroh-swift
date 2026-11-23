import Foundation
import IrohSwiftFFI

extension IrohNode {
    /// Get information about this node.
    ///
    /// - Returns: Node information including ID, relay URL, and connection status.
    /// - Throws: `IrohError.nodeClosed` if the node is closed,
    ///           `CancellationError` if the calling task was cancelled,
    ///           `IrohError.nodeCreationFailed` if the info cannot be retrieved.
    public func info() async throws -> NodeInfo {
        try ensureNotClosed()
        try Task.checkCancellation()

        let token = CancelTokenBox()
        let handle = self.handle

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                let box = Unmanaged.passRetained(
                    ContinuationBox<NodeInfo>(continuation)
                ).toOpaque()

                let callback = IrohNodeInfoAsyncCallback(
                    userdata: box,
                    on_success: { userdata, info in
                        let box = Unmanaged<ContinuationBox<NodeInfo>>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()

                        let nodeId = takeFFIString(info.node_id)

                        let relayUrl: String?
                        if info.relay_url != nil {
                            relayUrl = takeFFIString(info.relay_url)
                        } else {
                            relayUrl = nil
                        }

                        let directAddrs: [String]
                        if info.direct_addrs != nil {
                            directAddrs = takeFFIString(info.direct_addrs)
                                .split(separator: "\n")
                                .map(String.init)
                        } else {
                            directAddrs = []
                        }

                        let nodeInfo = NodeInfo(
                            nodeId: nodeId,
                            relayUrl: relayUrl,
                            isConnected: info.is_connected,
                            directAddrs: directAddrs,
                            dhtActive: info.dht_active
                        )
                        box.continuation.resume(returning: nodeInfo)
                    },
                    on_failure: { userdata, code, errorPtr in
                        let box = Unmanaged<ContinuationBox<NodeInfo>>
                            .fromOpaque(userdata!)
                            .takeRetainedValue()
                        let message = takeFFIString(errorPtr)
                        let error: Error
                        switch code {
                        case IrohOperationErrorCode_Cancelled:
                            error = CancellationError()
                        case IrohOperationErrorCode_NodeShutDown:
                            error = IrohError.nodeClosed
                        default:
                            error = IrohError.nodeCreationFailed(message)
                        }
                        box.continuation.resume(throwing: error)
                    }
                )

                iroh_node_info_async(handle.pointer, token.pointer, callback)
            }
        } onCancel: {
            token.cancel()
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

