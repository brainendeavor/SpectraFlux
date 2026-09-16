#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// Single source of truth: canonical TypeScript Seed
const SEED_DIR = path.resolve(__dirname, "../../../seeds/typescript");

function printUsage() {
  console.log(`
⚡ SpectraFlux TypeScript Bootstrapper

Usage:
  node bootstrap.mjs <name> [options]
  bun run bootstrap <name> [options]

Options:
  --path <dir>         Target directory (defaults to ./<name>)
  --mount <path>       Default HTTP mount path (e.g. /api/orders)
  --desc <text>        Fluxcell description
  --help               Show this help message
`);
}

async function main() {
  const args = process.argv.slice(2);
  if (args.length === 0 || args.includes("--help") || args.includes("-h")) {
    printUsage();
    process.exit(args.length === 0 ? 1 : 0);
  }

  const name = args[0];
  let targetPath = null;
  let mountPath = `/${name.replace(/^fluxcell-/, "")}`;
  let description = `${name} WebAssembly Fluxcell for SpectraFlux`;

  for (let i = 1; i < args.length; i++) {
    if (args[i] === "--path" && args[i + 1]) {
      targetPath = args[++i];
    } else if (args[i] === "--mount" && args[i + 1]) {
      mountPath = args[++i];
    } else if (args[i] === "--desc" && args[i + 1]) {
      description = args[++i];
    }
  }

  const targetDir = path.resolve(process.cwd(), targetPath || name);

  if (!fs.existsSync(SEED_DIR)) {
    console.error(`❌ Error: Seed directory not found at ${SEED_DIR}`);
    process.exit(1);
  }

  if (fs.existsSync(targetDir)) {
    console.error(`❌ Error: Target directory already exists: ${targetDir}`);
    process.exit(1);
  }

  console.log(`⚡ Scaffolding new TypeScript Fluxcell '${name}' from seed...`);
  console.log(`  Target: ${targetDir}`);
  console.log(`  Mount:  ${mountPath}`);

  copyDirRecursive(SEED_DIR, targetDir, {
    name,
    description,
    mountPath,
  });

  console.log(`\n✓ Successfully scaffolded '${name}'!`);
  console.log(`\nNext steps:`);
  console.log(`  cd ${path.relative(process.cwd(), targetDir) || "."}`);
  console.log(`  bun install        # or npm install`);
  console.log(`  bun run build      # or npm run build`);
  console.log(`  bun test           # or npm test`);
  console.log(`  bun run deploy --dev-upload`);
}

function copyDirRecursive(src, dest, vars) {
  fs.mkdirSync(dest, { recursive: true });
  const entries = fs.readdirSync(src, { withFileTypes: true });

  for (const entry of entries) {
    const srcPath = path.join(src, entry.name);
    const destPath = path.join(dest, entry.name);

    if (entry.isDirectory()) {
      if (entry.name === "node_modules" || entry.name === "build" || entry.name === ".git") {
        continue;
      }
      copyDirRecursive(srcPath, destPath, vars);
    } else if (entry.isFile()) {
      if (entry.name.endsWith(".wasm") || entry.name.endsWith(".sha256")) {
        continue;
      }
      let content = fs.readFileSync(srcPath, "utf8");
      // Parameterize placeholders
      content = content
        .replaceAll("fluxcell-seed-ts", vars.name)
        .replaceAll("fluxcell-ts-seed", vars.name)
        .replaceAll("Barebones Starter Fluxcell", vars.description)
        .replaceAll("/api/starter", vars.mountPath);

      fs.writeFileSync(destPath, content, "utf8");
      if (entry.name.endsWith(".sh")) {
        try {
          fs.chmodSync(destPath, 0o755);
        } catch (_) {}
      }
    }
  }
}

main().catch((err) => {
  console.error("Fatal error:", err);
  process.exit(1);
});
