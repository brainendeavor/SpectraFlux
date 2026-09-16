#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";

function printUsage() {
  console.log(`
⚡ SpectraFlux Fluxcell Deployment Tool

Usage:
  node deploy.mjs [options]
  bun run deploy [options]

Options:
  --dev-upload            Upload local .wasm directly to downstream chassis (Dev mode)
  --gateway <url>         SpectraGQL gateway URL (default: http://127.0.0.1:8000)
  --flux-url <url>        SpectraFlux chassis URL (default: http://127.0.0.1:8081)
  --mount <path>          HTTP mount path (e.g. /api/invoices)
  --name <name>           Fluxcell name (inferred from package.json if omitted)
  --token <token>         Deployment bearer token (or env SPECTRA_DEPLOY_TOKEN)
  --activate              Auto-activate fluxcell after staging (default: false)
  --status                Query chassis deployment & health status
  --lockdown              Trigger emergency lockdown cluster-wide
  --help                  Show this help message
`);
}

async function main() {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    printUsage();
    process.exit(0);
  }

  let gateway = process.env.SPECTRA_GATEWAY_URL || "http://127.0.0.1:8000";
  let fluxUrl = process.env.SPECTRA_FLUX_URL || "http://127.0.0.1:8081";
  let token = process.env.SPECTRA_DEPLOY_TOKEN || null;
  let mount = "/";
  let name = null;
  let devUpload = false;
  let activate = false;
  let statusMode = false;
  let lockdownMode = false;

  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--dev-upload") devUpload = true;
    else if (args[i] === "--activate") activate = true;
    else if (args[i] === "--status") statusMode = true;
    else if (args[i] === "--lockdown") lockdownMode = true;
    else if (args[i] === "--gateway" && args[i + 1]) gateway = args[++i];
    else if (args[i] === "--flux-url" && args[i + 1]) fluxUrl = args[++i];
    else if (args[i] === "--token" && args[i + 1]) token = args[++i];
    else if (args[i] === "--mount" && args[i + 1]) mount = args[++i];
    else if (args[i] === "--name" && args[i + 1]) name = args[++i];
  }

  // Handle status
  if (statusMode) {
    await queryStatus(fluxUrl);
    return;
  }

  // Handle lockdown
  if (lockdownMode) {
    await triggerLockdown(fluxUrl);
    return;
  }

  // Infer cell name from package.json
  if (!name) {
    const pkgPath = path.resolve(process.cwd(), "package.json");
    if (fs.existsSync(pkgPath)) {
      try {
        const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));
        name = pkg.name ? pkg.name.replace(/^@.*?\//, "") : "fluxcell-app";
      } catch (_) {
        name = "fluxcell-app";
      }
    } else {
      name = "fluxcell-app";
    }
  }

  // Dev Upload workflow
  if (devUpload) {
    const wasmPath = path.resolve(process.cwd(), "build/release.wasm");
    if (!fs.existsSync(wasmPath)) {
      console.error(`❌ Error: ${wasmPath} not found. Run 'bun run build' first.`);
      process.exit(1);
    }

    const wasmBytes = fs.readFileSync(wasmPath);
    const url = `${fluxUrl.replace(/\/$/, "")}/_flux/deployer/upload?name=${encodeURIComponent(name)}&mount_path=${encodeURIComponent(mount)}&auto_activate=${activate}`;

    console.log(`⚡ Direct Dev Upload: Uploading local .wasm to ${fluxUrl}...`);
    console.log(`  Name:  ${name}`);
    console.log(`  Mount: ${mount}`);

    const headers = { "Content-Type": "application/wasm" };
    if (token) headers["Authorization"] = `Bearer ${token}`;

    try {
      const resp = await fetch(url, { method: "POST", headers, body: wasmBytes });
      const text = await resp.text();
      if (resp.ok) {
        console.log(`✓ Successfully uploaded '${name}'! Response: ${text}`);
      } else {
        console.error(`❌ Dev upload rejected (HTTP ${resp.status}): ${text}`);
        process.exit(1);
      }
    } catch (err) {
      console.error(`❌ Failed to connect to SpectraFlux chassis: ${err.message}`);
      process.exit(1);
    }
    return;
  }

  // Mode B GraphQL mutation workflow
  console.log(`⚡ Mode B GraphQL deployment requires --dev-upload for local development or remote artifact parameters.`);
  console.log(`Example: bun run deploy --dev-upload --mount ${mount}`);
}

async function queryStatus(endpoint) {
  const url = `${endpoint.replace(/\/$/, "")}/_flux/deployer/status`;
  console.log(`⚡ Querying status from ${url}...`);

  try {
    const resp = await fetch(url);
    if (!resp.ok) {
      console.error(`❌ Status query failed (HTTP ${resp.status})`);
      process.exit(1);
    }
    const data = await resp.json();
    console.log(`\n─────────────────────────────────────────────────────────────`);
    console.log(`  SPECTRAFLUX CHASSIS STATUS & GOVERNANCE`);
    console.log(`─────────────────────────────────────────────────────────────`);
    if (data.guard) {
      console.log(`  External Deploy Enabled: ${data.guard.external_deploy_enabled}`);
      console.log(`  Dev Upload Enabled:      ${data.guard.dev_upload_enabled}`);
    }
    console.log(`\n  ACTIVE FLUXCELLS:`);
    const active = data.active_fluxcells || [];
    if (active.length === 0) {
      console.log(`    (None)`);
    } else {
      for (const cell of active) {
        console.log(`    • ${cell.name} v${cell.version || "0.1.0"} (Mount: ${cell.mount_path || "/"})`);
      }
    }
    console.log(`─────────────────────────────────────────────────────────────\n`);
  } catch (err) {
    console.error(`❌ Failed to query status: ${err.message}`);
    process.exit(1);
  }
}

async function triggerLockdown(endpoint) {
  const url = `${endpoint.replace(/\/$/, "")}/admin/api/v1/security/lockdown`;
  console.log(`🚨 TRIGGERING EMERGENCY LOCKDOWN on ${url}...`);

  try {
    const resp = await fetch(url, { method: "POST" });
    const text = await resp.text();
    if (resp.ok) {
      console.log(`🛑 LOCKDOWN SUCCESSFUL: Deployment ingress terminated cluster-wide.`);
      console.log(text);
    } else {
      console.error(`❌ Lockdown request failed (HTTP ${resp.status}): ${text}`);
      process.exit(1);
    }
  } catch (err) {
    console.error(`❌ Failed to trigger lockdown: ${err.message}`);
    process.exit(1);
  }
}

main().catch((err) => {
  console.error("Fatal error:", err);
  process.exit(1);
});
