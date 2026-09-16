#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawnSync } from "node:child_process";

async function main() {
  const args = process.argv.slice(2);
  const isDebug = args.includes("--debug");
  const target = isDebug ? "debug" : "release";

  console.log(`⚡ Compiling AssemblyScript Fluxcell (${target})...`);

  // Ensure build output directory exists
  const buildDir = path.resolve(process.cwd(), "build");
  fs.mkdirSync(buildDir, { recursive: true });

  // Resolve asc binary: check local node_modules, SDK node_modules, or global
  let ascBin = path.resolve(process.cwd(), "node_modules/.bin/asc");
  if (!fs.existsSync(ascBin)) {
    // Check if running from SDK or parent workspace
    ascBin = path.resolve(process.cwd(), "../../sdks/typescript/node_modules/.bin/asc");
  }

  const ascArgs = [
    "assembly/index.ts",
    "--target",
    target,
  ];

  let res;
  if (fs.existsSync(ascBin)) {
    res = spawnSync(ascBin, ascArgs, { stdio: "inherit", shell: true });
  } else {
    // Fallback to npx/bunx asc
    const runner = typeof process.versions.bun !== "undefined" ? "bunx" : "npx";
    res = spawnSync(runner, ["asc", ...ascArgs], { stdio: "inherit", shell: true });
  }

  if (res.status !== 0) {
    console.error(`❌ Compilation failed with exit code ${res.status}`);
    process.exit(res.status || 1);
  }

  const wasmPath = path.join(buildDir, `${target}.wasm`);
  if (!fs.existsSync(wasmPath)) {
    console.error(`❌ Output wasm file not found at ${wasmPath}`);
    process.exit(1);
  }

  const bytes = fs.readFileSync(wasmPath);
  const sizeKb = (bytes.length / 1024).toFixed(2);
  const hash = crypto.createHash("sha256").update(bytes).digest("hex");

  const shaPath = `${wasmPath}.sha256`;
  fs.writeFileSync(shaPath, `${hash}  ${path.basename(wasmPath)}\n`, "utf8");

  console.log(`✓ Built: ${wasmPath} (${sizeKb} KB)`);
  console.log(`✓ SHA-256: ${hash}`);
}

main().catch((err) => {
  console.error("Fatal error:", err);
  process.exit(1);
});
