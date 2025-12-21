#!/bin/bash
# Build script for creating XCFramework from Rust static library
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
RUST_DIR="$PROJECT_ROOT/rust"
OUT_DIR="$PROJECT_ROOT/build"
XCFRAMEWORK="$PROJECT_ROOT/IrohSwiftFFI.xcframework"

echo "Building iroh-swift FFI..."

# Rust targets for Apple platforms (aarch64 only)
TARGETS=(
    "aarch64-apple-ios"
    "aarch64-apple-ios-sim"
    "aarch64-apple-darwin"
)

# Ensure targets are installed
echo "Installing Rust targets..."
for target in "${TARGETS[@]}"; do
    rustup target add "$target" 2>/dev/null || true
done

# Build for all targets with correct deployment targets
echo "Building Rust library for all targets..."
for target in "${TARGETS[@]}"; do
    echo "  Building for $target..."

    # Set deployment targets to match Swift package (26.0)
    case "$target" in
        *-ios*)
            export IPHONEOS_DEPLOYMENT_TARGET=26.0
            ;;
        *-darwin*)
            export MACOSX_DEPLOYMENT_TARGET=26.0
            ;;
    esac

    cargo build --manifest-path "$RUST_DIR/Cargo.toml" \
        --target "$target" \
        --release
done

# Generate C header
echo "Generating C header..."
cd "$RUST_DIR"
cbindgen --config cbindgen.toml \
    --crate iroh-swift-ffi \
    --output "$PROJECT_ROOT/include/iroh_swift.h"
cd "$PROJECT_ROOT"

# Create output directories
echo "Creating output directories..."
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"/{ios-device,ios-simulator,macos}

# iOS Device (arm64)
echo "Creating iOS device library..."
cp "$RUST_DIR/target/aarch64-apple-ios/release/libiroh_swift.a" \
   "$OUT_DIR/ios-device/"

# iOS Simulator (arm64 only)
echo "Copying iOS simulator library..."
cp "$RUST_DIR/target/aarch64-apple-ios-sim/release/libiroh_swift.a" \
   "$OUT_DIR/ios-simulator/"

# macOS (arm64 only)
echo "Copying macOS library..."
cp "$RUST_DIR/target/aarch64-apple-darwin/release/libiroh_swift.a" \
   "$OUT_DIR/macos/"

# Create XCFramework
echo "Creating XCFramework..."
rm -rf "$XCFRAMEWORK"
xcodebuild -create-xcframework \
    -library "$OUT_DIR/ios-device/libiroh_swift.a" \
    -headers "$PROJECT_ROOT/include" \
    -library "$OUT_DIR/ios-simulator/libiroh_swift.a" \
    -headers "$PROJECT_ROOT/include" \
    -library "$OUT_DIR/macos/libiroh_swift.a" \
    -headers "$PROJECT_ROOT/include" \
    -output "$XCFRAMEWORK"

echo ""
echo "XCFramework created at: $XCFRAMEWORK"
echo ""

# Show size info
echo "Library sizes:"
du -h "$OUT_DIR"/*/libiroh_swift.a
echo ""
du -sh "$XCFRAMEWORK"
