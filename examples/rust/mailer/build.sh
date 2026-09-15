#!/usr/bin/env bash
set -euo pipefail

echo "==> Building Fluxcell: Invoice Mailer"
cargo build --target wasm32-wasip1 --release || cargo build --target wasm32-unknown-unknown --release

WASM_PATH="target/wasm32-wasip1/release/fluxcell_invoice_mailer.wasm"
if [ ! -f "$WASM_PATH" ]; then
    WASM_PATH="target/wasm32-unknown-unknown/release/fluxcell_invoice_mailer.wasm"
fi

if [ -f "$WASM_PATH" ]; then
    SHA=$(shasum -a 256 "$WASM_PATH" | cut -d ' ' -f 1)
    echo "=================================================="
    echo " Artifact Built Successfully!"
    echo " Path:   $WASM_PATH"
    echo " SHA256: $SHA"
    echo "=================================================="
    echo ""
    echo "To deploy via SpectraGQL Mode B GraphQL Mutation:"
    echo "mutation {"
    echo "  deployFluxcell(input: {"
    echo "    name: \"invoice-mailer\","
    echo "    artifactUrl: \"https://github.com/your-org/fluxcell-invoice-mailer/releases/download/v0.1.0/invoice-mailer.wasm\","
    echo "    sha256: \"$SHA\","
    echo "    mountPath: \"/api/invoices\""
    echo "  }) {"
    echo "    commandId"
    echo "    status"
    echo "  }"
    echo "}"
fi
