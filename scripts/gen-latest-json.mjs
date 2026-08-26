#!/usr/bin/env node
// Generate the Tauri updater manifest (latest.json) from the signed build
// artifacts. Scans the artifacts dir for `.sig` files, maps each to a platform
// key + public download URL, and writes latest.json alongside them.
//
// Usage: node scripts/gen-latest-json.mjs <version> <owner/repo> <artifactsDir>

import fs from "node:fs";
import path from "node:path";

/**
 * Map an artifact filename to its updater platform key(s).
 *
 * The manifest describes what the *installed* updater may download and apply,
 * and that is the only thing it may ever describe. A portable ZIP reaching this
 * list would mean an installed ArcScan downloading a portable build and handing
 * it to the NSIS updater. The failure mode of an update path is not something to
 * leave to a filename coincidence.
 *
 * Two independent things keep it out. Only assets with a matching `.sig` are
 * considered at all, and a portable ZIP is never signed as an updater artifact.
 * And this refuses anything named portable outright, before any suffix is
 * examined, so no future asset name can creep in through a suffix match.
 */
export function platformsFor(name) {
  const lower = name.toLowerCase();

  // Never an updater payload, whatever else it looks like.
  if (lower.includes("portable")) return [];

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

/**
 * Build the manifest's `platforms` map from the files in an artifacts
 * directory. Separated from the writing so its rules are testable against a
 * fixture rather than only against a real signed release.
 */
export function buildPlatforms(files, readSignature, versionArg, repo) {
  const platforms = {};
  for (const sig of files.filter((f) => f.endsWith(".sig"))) {
    const asset = sig.replace(/\.sig$/, "");
    if (!files.includes(asset)) continue;
    const keys = platformsFor(asset);
    if (keys.length === 0) continue;
    const signature = readSignature(sig);
    const url = `https://github.com/${repo}/releases/download/${versionArg}/${encodeURIComponent(asset)}`;
    for (const key of keys) {
      // Prefer the NSIS installer over the MSI if both produced signatures.
      if (platforms[key] && asset.toLowerCase().endsWith(".msi")) continue;
      platforms[key] = { signature, url };
    }
  }
  return platforms;
}

function main(versionArg, repo, dirArg) {
  const version = versionArg.replace(/^v/, "");
  const dir = path.resolve(dirArg);
  const files = fs.readdirSync(dir);

  const platforms = buildPlatforms(
    files,
    (sig) => fs.readFileSync(path.join(dir, sig), "utf8").trim(),
    versionArg,
    repo,
  );

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
}

// Run only when invoked as a script, so the rules above can be imported and
// tested without writing a manifest.
if (process.argv[1] && path.basename(process.argv[1]) === "gen-latest-json.mjs") {
  const [, , versionArg, repo, dirArg] = process.argv;
  if (!versionArg || !repo || !dirArg) {
    console.error("usage: gen-latest-json.mjs <version> <owner/repo> <artifactsDir>");
    process.exit(1);
  }
  main(versionArg, repo, dirArg);
}
