import Foundation
import IrohSwiftFFI

/// Owns an `IrohCancelToken` for one cancellable FFI operation.
///
/// Created before the operation starts and captured by the
/// `withTaskCancellationHandler` `onCancel` closure, so the token outlives
/// every call to `cancel()`; it is freed in `deinit`. The Rust operation holds
/// its own reference, so freeing never races with the transfer.
final class CancelTokenBox: @unchecked Sendable {
    let pointer: UnsafeMutablePointer<IrohCancelToken>

    init() {
        pointer = iroh_cancel_token_new()
    }

    /// Cancel the operation. Thread-safe and idempotent.
    func cancel() {
        iroh_cancel_token_cancel(pointer)
    }

    deinit {
        iroh_cancel_token_free(pointer)
    }
}

/// Call `body` with a C string for `string`, or with nil when `string` is nil.
func withOptionalCString<R>(
    _ string: String?,
    _ body: (UnsafePointer<CChar>?) throws -> R
) rethrows -> R {
    if let string {
        return try string.withCString { try body($0) }
    }
    return try body(nil)
}

/// Call `body` with a C array of C strings (nil base address when empty).
func withCStringArray<R>(
    _ strings: [String],
    _ body: (UnsafePointer<UnsafePointer<CChar>?>?, Int) throws -> R
) rethrows -> R {
    if strings.isEmpty {
        return try body(nil, 0)
    }
    let owned = strings.map { strdup($0) }
    defer { owned.forEach { free($0) } }
    let pointers: [UnsafePointer<CChar>?] = owned.map { $0.map { UnsafePointer($0) } }
    return try pointers.withUnsafeBufferPointer { try body($0.baseAddress, strings.count) }
}

/// Take ownership of a string the FFI allocated: copy it and free the original.
func takeFFIString(_ pointer: UnsafePointer<CChar>?) -> String {
    guard let pointer else { return "" }
    let string = String(cString: pointer)
    iroh_string_free(UnsafeMutablePointer(mutating: pointer))
    return string
}
