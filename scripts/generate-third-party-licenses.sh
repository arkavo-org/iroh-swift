#!/bin/bash
# Generate (or verify) THIRD-PARTY-LICENSES.md from rust/Cargo.lock using cargo-about.
#
# Usage:
#   ./scripts/generate-third-party-licenses.sh           # regenerate THIRD-PARTY-LICENSES.md
#   ./scripts/generate-third-party-licenses.sh --check   # fail if THIRD-PARTY-LICENSES.md is out of date
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
RUST_DIR="$PROJECT_ROOT/rust"
OUTPUT_FILE="$PROJECT_ROOT/THIRD-PARTY-LICENSES.md"

CHECK_MODE=0
if [ "${1:-}" = "--check" ]; then
    CHECK_MODE=1
elif [ -n "${1:-}" ]; then
    echo "Usage: $0 [--check]" >&2
    exit 2
fi

if ! command -v cargo-about >/dev/null 2>&1; then
    echo "error: cargo-about is not installed." >&2
    echo "Install it with: brew install cargo-about  (or: cargo install --locked cargo-about)" >&2
    exit 1
fi

TMP_OUT="$(mktemp "${TMPDIR:-/tmp}/third-party-licenses.XXXXXX")"
trap 'rm -f "$TMP_OUT"' EXIT

# --frozen enforces both --locked (Cargo.lock unchanged) and --offline
# (deterministic generation from cached crates without network access).
# Ensure crates are fetched locally before running (e.g. `cargo fetch --locked`
# in `rust/`).
cargo about generate \
    --frozen \
    --fail \
    --config "$RUST_DIR/about.toml" \
    --manifest-path "$RUST_DIR/Cargo.toml" \
    --output-file "$TMP_OUT" \
    "$RUST_DIR/about.hbs"

if [ "$CHECK_MODE" -eq 1 ]; then
    if [ ! -f "$OUTPUT_FILE" ]; then
        echo "error: $OUTPUT_FILE does not exist. Run ./scripts/generate-third-party-licenses.sh" >&2
        exit 1
    fi
    if ! diff -u "$OUTPUT_FILE" "$TMP_OUT"; then
        echo "" >&2
        echo "error: THIRD-PARTY-LICENSES.md is out of date with rust/Cargo.lock." >&2
        echo "Run ./scripts/generate-third-party-licenses.sh and commit the result." >&2
        exit 1
    fi
    echo "THIRD-PARTY-LICENSES.md is up to date."
else
    mv "$TMP_OUT" "$OUTPUT_FILE"
    echo "Generated $OUTPUT_FILE"
fi
