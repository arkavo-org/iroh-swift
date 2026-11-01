import Foundation
import IrohSwiftFFI
import Synchronization

extension IrohDoc {
    /// Subscribe to document events.
    ///
    /// Returns an async stream of events that occur on this document,
    /// including local and remote insertions, content availability,
    /// and peer activity.
    ///
    /// Example usage:
    /// ```swift
    /// for try await event in try doc.subscribe() {
    ///     switch event {
    ///     case .insertLocal(let entry):
    ///         print("Local write: \(entry.keyString ?? "?")")
    ///     case .insertRemote(let from, let entry):
    ///         print("Remote from \(from): \(entry.keyString ?? "?")")
    ///     case .neighborUp(let peer):
    ///         print("Peer joined: \(peer)")
    ///     case .neighborDown(let peer):
    ///         print("Peer left: \(peer)")
    ///     case .syncFinished(let peer):
    ///         print("Sync complete with: \(peer)")
    ///     case .contentReady(let hash):
    ///         print("Content ready: \(hash)")
    ///     case .pendingContentReady:
    ///         print("All pending content ready")
    ///     }
    /// }
    /// ```
    ///
    /// - Returns: An async throwing stream of document events.
    /// - Throws: `IrohError.docClosed` if the document is closed.
    public func subscribe() throws -> AsyncThrowingStream<DocEvent, Error> {
        try ensureNotClosed()

        return AsyncThrowingStream(bufferingPolicy: .bufferingNewest(100)) { continuation in
            let context = SubscriptionContext(continuation: continuation)
            let contextPtr = Unmanaged.passRetained(context).toOpaque()

            // Set up cancellation handler
            continuation.onTermination = { @Sendable termination in
                // Cancel the FFI subscription when the stream is terminated
                context.cancel()
            }

            let callback = IrohDocSubscribeCallback(
                userdata: contextPtr,
                on_event: { userdata, event in
                    // takeUnretainedValue - don't consume, more events coming
                    let ctx = Unmanaged<SubscriptionContext>
                        .fromOpaque(userdata!)
                        .takeUnretainedValue()

                    let docEvent = DocEvent.from(event)
                    let eventType = event.event_type.rawValue
                    iroh_doc_event_free(event)
                    // A malformed or newer event type is dropped, not fatal
                    guard let docEvent else {
                        IrohLogger.node.warning("subscribe: dropped document event of type \(eventType)")
                        return
                    }
                    ctx.continuation.yield(docEvent)
                },
                on_complete: { userdata in
                    // takeRetainedValue - consume on terminal
                    let ctx = Unmanaged<SubscriptionContext>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    ctx.continuation.finish()
                },
                on_failure: { userdata, errorPtr in
                    // takeRetainedValue - consume on terminal
                    let ctx = Unmanaged<SubscriptionContext>
                        .fromOpaque(userdata!)
                        .takeRetainedValue()
                    let message = String(cString: errorPtr!)
                    iroh_string_free(UnsafeMutablePointer(mutating: errorPtr))
                    ctx.continuation.finish(throwing: IrohError.docSubscribeFailed(message))
                }
            )

            // Callbacks can end the stream on a Tokio thread before this returns;
            // setHandle then cancels (and frees) the handle at once.
            let subHandle = iroh_doc_subscribe(handle.pointer, callback)
            context.setHandle(SubscriptionHandleWrapper(pointer: subHandle))
        }
    }
}

// MARK: - Subscription Context

/// Internal context for managing a subscription.
///
/// The handle is set on the subscribing thread, while `cancel()` can run from
/// `onTermination` on a Tokio thread, before or after that. The lock orders the
/// two, and `iroh_subscription_cancel` (which frees the handle) runs exactly once.
private final class SubscriptionContext: Sendable {
    let continuation: AsyncThrowingStream<DocEvent, Error>.Continuation
    private let state = Mutex(State())

    private struct State {
        var handle: SubscriptionHandleWrapper?
        var cancelled = false
    }

    init(continuation: AsyncThrowingStream<DocEvent, Error>.Continuation) {
        self.continuation = continuation
    }

    /// Store the handle, or cancel it at once if the stream has already ended.
    func setHandle(_ handle: SubscriptionHandleWrapper) {
        let cancelNow = state.withLock { state in
            if state.cancelled { return true }
            state.handle = handle
            return false
        }
        if cancelNow {
            iroh_subscription_cancel(handle.pointer)
        }
    }

    func cancel() {
        let handle = state.withLock { state in
            state.cancelled = true
            defer { state.handle = nil }
            return state.handle
        }
        if let handle {
            iroh_subscription_cancel(handle.pointer)
        }
    }
}

/// Sendable wrapper for the subscription handle pointer.
private struct SubscriptionHandleWrapper: @unchecked Sendable {
    let pointer: UnsafeMutablePointer<IrohSubscriptionHandle>?
}
