#!/usr/bin/env node
// Check the real latest.json immediately before publication.
//
// The generator has unit tests, but this is defense in depth against wrong
// workflow arguments, stale signed artifacts, and hand-edited output. Installed
// ArcScan may act on every URL in this file without a person inspecting it, so
// the platform set, release version, architecture, and payload kind are exact.
//
//   node scripts/check-latest-json.mjs <latest.json> <expected version or tag>

import { readFileSync } from "node:fs";

const file = process.argv[2];
const expectedArg = process.argv[3];
if (!file || !expectedArg) {
  console.error("usage: check-latest-json.mjs <path to latest.json> <expected version or v-tag>");
  process.exit(1);
}

const expectedVersion = expectedArg.replace(/^v/, "");
if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(expectedVersion)) {
  console.error(`expected version is not semantic: ${expectedArg}`);
  process.exit(1);
}

let manifest;
try {
  manifest = JSON.parse(readFileSync(file, "utf8"));
} catch (error) {
  console.error(`could not read ${file}: ${error.message}`);
  process.exit(1);
}

const problems = [];
const REQUIRED_KEYS = [
  "darwin-aarch64",
  "darwin-x86_64",
  "windows-aarch64",
  "windows-x86_64",
];
const EXPECTED_ASSET = {
  // Tauri's macOS updater payload is intentionally unversioned; the release-tag
  // segment above is what binds it to the expected version. The human-facing
  // universal DMG remains versioned and is asserted separately in the workflow.
  "darwin-aarch64": "ArcScan.app.tar.gz",
  "darwin-x86_64": "ArcScan.app.tar.gz",
  "windows-aarch64": `ArcScan_${expectedVersion}_arm64-setup.exe`,
  "windows-x86_64": `ArcScan_${expectedVersion}_x64-setup.exe`,
};

if (manifest?.version !== expectedVersion) {
  problems.push(`manifest version is ${JSON.stringify(manifest?.version)}, expected ${expectedVersion}`);
}

const platforms =
  manifest?.platforms && typeof manifest.platforms === "object" && !Array.isArray(manifest.platforms)
    ? manifest.platforms
    : {};
const actualKeys = Object.keys(platforms).sort();
if (JSON.stringify(actualKeys) !== JSON.stringify(REQUIRED_KEYS)) {
  problems.push(
    `platform keys are ${JSON.stringify(actualKeys)}, expected exactly ${JSON.stringify(REQUIRED_KEYS)}`,
  );
}

for (const key of actualKeys) {
  const entry = platforms[key];
  let decodedUrl = "";
  try {
    decodedUrl = decodeURIComponent(entry?.url ?? "");
  } catch {
    problems.push(`platform ${key} has a URL that cannot be decoded: ${entry?.url ?? ""}`);
    continue;
  }

  if (/portable/i.test(decodedUrl)) {
    problems.push(`platform ${key} points at a Portable asset: ${decodedUrl}`);
  }
  if (typeof entry?.signature !== "string" || entry.signature.trim() === "") {
    problems.push(`platform ${key} has no non-empty updater signature`);
  }

  let parsed;
  try {
    parsed = new URL(decodedUrl);
  } catch {
    problems.push(`platform ${key} has an invalid URL: ${decodedUrl}`);
    continue;
  }
  if (parsed.protocol !== "https:" || parsed.hostname !== "github.com") {
    problems.push(`platform ${key} does not use an HTTPS GitHub release URL: ${decodedUrl}`);
  }

  const expectedTagPath = `/releases/download/v${expectedVersion}/`;
  if (!parsed.pathname.includes(expectedTagPath)) {
    problems.push(
      `platform ${key} does not target release v${expectedVersion}: ${decodedUrl}`,
    );
  }

  const asset = pathBasename(parsed.pathname);
  const expectedAsset = EXPECTED_ASSET[key];
  if (!expectedAsset) {
    problems.push(`platform ${key} is not an Installed updater platform`);
  } else if (asset !== expectedAsset) {
    problems.push(
      `platform ${key} points at ${asset || "no asset"}, expected ${expectedAsset}`,
    );
  }
}

function pathBasename(pathname) {
  const slash = pathname.lastIndexOf("/");
  return slash >= 0 ? pathname.slice(slash + 1) : pathname;
}

console.log(`${file}`);
console.log(`  expected  ${expectedVersion}`);
console.log(`  version   ${manifest?.version}`);
console.log(`  platforms ${actualKeys.join(", ")}`);
for (const [key, entry] of Object.entries(platforms)) {
  let url = entry?.url ?? "";
  try {
    url = decodeURIComponent(url);
  } catch {
    // Keep the raw value in the report; the failure is already recorded above.
  }
  console.log(`    ${key} -> ${url}`);
}

if (problems.length > 0) {
  console.error("");
  for (const problem of problems) console.error(`  FAIL  ${problem}`);
  process.exit(1);
}
console.log("\nEvery expectation held.");
