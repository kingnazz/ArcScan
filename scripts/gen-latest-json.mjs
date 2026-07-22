#!/usr/bin/env node
// Generate the Tauri updater manifest (latest.json) from the signed build
// artifacts. Scans the artifacts dir for `.sig` files, maps each to a platform
// key + public download URL, and writes latest.json alongside them.
//
// Usage: node scripts/gen-latest-json.mjs <version> <owner/repo> <artifactsDir>

import fs from "node:fs";
import path from "node:path";

const [, , versionArg, repo, dirArg] = process.argv;
if (!versionArg || !repo || !dirArg) {
  console.error("usage: gen-latest-json.mjs <version> <owner/repo> <artifactsDir>");
  process.exit(1);
}

const version = versionArg.replace(/^v/, "");
const dir = path.resolve(dirArg);
const files = fs.readdirSync(dir);

// Map an artifact filename to its updater platform key(s).
function platformsFor(name) {
  const lower = name.toLowerCase();
  if (lower.endsWith(".app.tar.gz")) {
    // A universal macOS build serves both architectures.
    return ["darwin-x86_64", "darwin-aarch64"];
  }
  if (lower.endsWith("-setup.exe") || lower.endsWith(".nsis.zip") || lower.endsWith(".msi")) {
    if (lower.includes("arm64") || lower.includes("aarch64")) return ["windows-aarch64"];
    return ["windows-x86_64"];
  }
  return [];
}

const platforms = {};
for (const sig of files.filter((f) => f.endsWith(".sig"))) {
  const asset = sig.replace(/\.sig$/, "");
  if (!files.includes(asset)) continue;
  const keys = platformsFor(asset);
  if (keys.length === 0) continue;
  const signature = fs.readFileSync(path.join(dir, sig), "utf8").trim();
  const url = `https://github.com/${repo}/releases/download/${versionArg}/${encodeURIComponent(asset)}`;
  for (const key of keys) {
    // Prefer the NSIS installer over the MSI if both produced signatures.
    if (platforms[key] && asset.toLowerCase().endsWith(".msi")) continue;
    platforms[key] = { signature, url };
  }
}

if (Object.keys(platforms).length === 0) {
  console.error("No signed updater artifacts (.sig) found — is TAURI_SIGNING_PRIVATE_KEY set?");
  process.exit(1);
}

const manifest = {
  version,
  notes: `ArcScan ${versionArg}`,
  pub_date: new Date().toISOString(),
  platforms,
};

fs.writeFileSync(path.join(dir, "latest.json"), JSON.stringify(manifest, null, 2));
console.log("Wrote latest.json:\n", JSON.stringify(manifest, null, 2));
