// swift-tools-version: 6.2
import PackageDescription

let version = "0.1.0"
// Checksum is updated by release automation
let checksum = "CHECKSUM_PLACEHOLDER"

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
        // For local development, use the local XCFramework
        // For releases, this will be replaced with a URL-based binary target
        .binaryTarget(
            name: "IrohSwiftFFI",
            path: "IrohSwiftFFI.xcframework"
        ),
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
