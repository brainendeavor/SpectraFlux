#!/usr/bin/env bash
set -euo pipefail

echo "⚡ Building TypeScript Fluxcell..."

if command -v bun &> /dev/null; then
  bun run build
else
  npm run build
fi

WASM_FILE="build/release.wasm"
if [ -f "$WASM_FILE" ]; then
  mkdir -p dist
  cp "$WASM_FILE" "dist/release.wasm"
  if [ -f "$WASM_FILE.sha256" ]; then
    cp "$WASM_FILE.sha256" "dist/release.wasm.sha256"
  fi
  SIZE=$(du -h "$WASM_FILE" | cut -f1)
  echo "✓ Successfully built $WASM_FILE ($SIZE)"
fi
