#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:-wasm32-wasip1}"

echo "⚡ Compiling Fluxcell to $TARGET..."
cargo build --target "$TARGET" --release

WASM_FILE=$(find target/"$TARGET"/release -maxdepth 1 -name "*.wasm" ! -name "*.*.wasm" | head -n 1)
WASM_NAME=$(basename "$WASM_FILE")

mkdir -p dist
cp "$WASM_FILE" "dist/$WASM_NAME"

if command -v sha256sum &> /dev/null; then
  sha256sum "dist/$WASM_NAME" > "dist/$WASM_NAME.sha256"
elif command -v shasum &> /dev/null; then
  shasum -a 256 "dist/$WASM_NAME" > "dist/$WASM_NAME.sha256"
fi

SIZE=$(du -h "dist/$WASM_NAME" | cut -f1)
echo "✓ Successfully built dist/$WASM_NAME ($SIZE)"
