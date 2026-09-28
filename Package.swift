// swift-tools-version: 6.2
import PackageDescription
import Foundation

let version = "0.5.0"

// Checksum is updated by release automation
let checksum = "441b29b038abb1e7c9ac8d135480423f41581d9634ef9dc7eae0857e57199394"

// Check if using local development mode
// Set IROH_LOCAL_DEV=1 environment variable to use local XCFramework
// Use local binary if XCFramework exists (local dev) or env var is set
// The existence check must be anchored at the package directory: the manifest
// is not always evaluated with it as the working directory (Xcode, or a
// consumer depending on this package by path), and a relative check silently
// falls back to the released artifact.
let useLocalBinary = ProcessInfo.processInfo.environment["IROH_LOCAL_DEV"] != nil
    || FileManager.default.fileExists(atPath: Context.packageDirectory + "/IrohSwiftFFI.xcframework")

// Binary target configuration
let binaryTarget: Target = useLocalBinary
    ? .binaryTarget(
        name: "IrohSwiftFFI",
        path: "IrohSwiftFFI.xcframework"
    )
    : .binaryTarget(
        name: "IrohSwiftFFI",
        url: "https://github.com/arkavo-org/iroh-swift/releases/download/\(version)/IrohSwiftFFI.xcframework.zip",
        checksum: checksum
    )

let package = Package(
    name: "IrohSwift",
    platforms: [
        .iOS(.v26),
        .macOS(.v26)
    ],
    products: [
        .library(
            name: "IrohSwift",
            targets: ["IrohSwift"]
        ),
        .executable(
            name: "iroh-cli",
            targets: ["IrohCLI"]
        ),
    ],
    targets: [
        binaryTarget,
        .target(
            name: "IrohSwift",
            dependencies: ["IrohSwiftFFI"],
            path: "Sources/IrohSwift",
            swiftSettings: [
                .enableExperimentalFeature("StrictConcurrency")
            ],
            linkerSettings: [
                .linkedFramework("SystemConfiguration"),
                .linkedFramework("Security"),
                // The iOS Rust library monitors network paths with nw_path_*
                // (rustc `--print native-static-libs` lists -framework Network
                // for the iOS and iOS Simulator targets only).
                .linkedFramework("Network", .when(platforms: [.iOS])),
                .linkedLibrary("resolv"),
            ]
        ),
        .testTarget(
            name: "IrohSwiftTests",
            dependencies: ["IrohSwift"],
            path: "Tests/IrohSwiftTests"
        ),
        .executableTarget(
            name: "IrohCLI",
            dependencies: ["IrohSwift"],
            path: "Sources/IrohCLI"
        ),
    ]
)
