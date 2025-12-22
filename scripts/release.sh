#!/bin/bash
# Release automation script for iroh-swift
# This script:
# 1. Builds the XCFramework
# 2. Creates a zip archive
# 3. Computes SHA256 checksum
# 4. Updates Package.swift with the checksum

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

# Read version from VERSION file
VERSION=$(cat VERSION)
echo "Releasing version: $VERSION"

# Build the XCFramework
echo "Building XCFramework..."
./scripts/build-xcframework.sh

# Create zip archive
XCFRAMEWORK_PATH="IrohSwiftFFI.xcframework"
ZIP_NAME="IrohSwiftFFI.xcframework.zip"

if [ ! -d "$XCFRAMEWORK_PATH" ]; then
    echo "Error: XCFramework not found at $XCFRAMEWORK_PATH"
    exit 1
fi

echo "Creating zip archive..."
rm -f "$ZIP_NAME"
zip -r -X "$ZIP_NAME" "$XCFRAMEWORK_PATH"

# Compute SHA256 checksum
echo "Computing checksum..."
CHECKSUM=$(shasum -a 256 "$ZIP_NAME" | awk '{print $1}')
echo "Checksum: $CHECKSUM"

# Update Package.swift with the new checksum
echo "Updating Package.swift..."
sed -i '' "s/let checksum = \".*\"/let checksum = \"$CHECKSUM\"/" Package.swift

echo ""
echo "Release preparation complete!"
echo "  Version: $VERSION"
echo "  Checksum: $CHECKSUM"
echo "  Artifact: $ZIP_NAME"
echo ""
echo "Next steps:"
echo "  1. Commit the updated Package.swift"
echo "  2. Create a git tag: git tag v$VERSION"
echo "  3. Push the tag: git push origin v$VERSION"
echo "  4. Create a GitHub release and upload $ZIP_NAME"
