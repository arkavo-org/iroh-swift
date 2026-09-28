#!/bin/bash
# Build script for creating XCFramework from Rust static library
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
RUST_DIR="$PROJECT_ROOT/rust"
OUT_DIR="$PROJECT_ROOT/build"
XCFRAMEWORK="$PROJECT_ROOT/IrohSwiftFFI.xcframework"

echo "Building iroh-swift FFI..."

# Rust targets for Apple platforms.
# The iOS Simulator and macOS slices are universal (arm64 + x86_64) because
# Xcode builds `generic/platform=iOS Simulator` and `generic/platform=macOS`
# for both architectures; an arm64-only slice fails to link for x86_64.
# The iOS device slice is arm64 only.
# Note: `x86_64-apple-ios` is Rust's (simulator-only) x86_64 iOS target.
TARGETS=(
    "aarch64-apple-ios"
    "aarch64-apple-ios-sim"
    "x86_64-apple-ios"
    "aarch64-apple-darwin"
    "x86_64-apple-darwin"
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

lib() {
    echo "$RUST_DIR/target/$1/release/libiroh_swift.a"
}

# iOS Device (arm64)
echo "Creating iOS device library (arm64)..."
cp "$(lib aarch64-apple-ios)" "$OUT_DIR/ios-device/"

# iOS Simulator (arm64 + x86_64)
echo "Creating universal iOS simulator library (arm64 + x86_64)..."
lipo -create \
    "$(lib aarch64-apple-ios-sim)" \
    "$(lib x86_64-apple-ios)" \
    -output "$OUT_DIR/ios-simulator/libiroh_swift.a"

# macOS (arm64 + x86_64)
echo "Creating universal macOS library (arm64 + x86_64)..."
lipo -create \
    "$(lib aarch64-apple-darwin)" \
    "$(lib x86_64-apple-darwin)" \
    -output "$OUT_DIR/macos/libiroh_swift.a"

# Fail if a slice lacks an architecture, so a release can never ship a slice
# that forces consumers to exclude an architecture.
echo "Verifying library architectures..."
check_archs() {
    local slice="$1" expected="$2" actual
    actual="$(lipo -archs "$OUT_DIR/$slice/libiroh_swift.a" | tr ' ' '\n' | sort | xargs)"
    printf '  %-14s %s\n' "$slice" "$actual"
    if [ "$actual" != "$expected" ]; then
        echo "error: $slice has architectures '$actual', expected '$expected'" >&2
        exit 1
    fi
}
check_archs ios-device "arm64"
check_archs ios-simulator "arm64 x86_64"
check_archs macos "arm64 x86_64"

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
