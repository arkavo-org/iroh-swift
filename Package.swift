// swift-tools-version: 6.2
import PackageDescription
import Foundation

let version = "0.2.5"

// Checksum is updated by release automation
let checksum = "3488cc2194afdd0cd4f4bba92b97d8c989c7247712fd04e13f70428645098622"

// Check if using local development mode
// Set IROH_LOCAL_DEV=1 environment variable to use local XCFramework
// Use local binary if XCFramework exists (local dev) or env var is set
let useLocalBinary = ProcessInfo.processInfo.environment["IROH_LOCAL_DEV"] != nil
    || FileManager.default.fileExists(atPath: "IrohSwiftFFI.xcframework")

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
