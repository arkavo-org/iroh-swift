import Foundation
import IrohSwiftFFI

/// Take ownership of a string the FFI allocated: copy it and free the original.
func takeFFIString(_ pointer: UnsafePointer<CChar>?) -> String {
    guard let pointer else { return "" }
    let string = String(cString: pointer)
    iroh_string_free(UnsafeMutablePointer(mutating: pointer))
    return string
}
