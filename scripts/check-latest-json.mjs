#!/usr/bin/env node
// Check a generated latest.json before it is published.
//
// gen-latest-json's rules have their own tests. This reads the real manifest
// that is about to go into a GitHub release and asserts the same properties
// again, because the file that matters is the one being uploaded, not the
// function that produced it: a wrong workflow argument, a stale artifact left in
// the directory, or a hand edit would all slip past a unit test.
//
// latest.json is the one file an installed ArcScan downloads and acts on without
// a person looking at it, so what it names must be an installer the NSIS updater
// can actually apply -- never a portable ZIP.
//
//   node scripts/check-latest-json.mjs <path to latest.json>

import { readFileSync } from "node:fs";

const file = process.argv[2];
if (!file) {
  console.error("usage: check-latest-json.mjs <path to latest.json>");
  process.exit(1);
}

let manifest;
try {
  manifest = JSON.parse(readFileSync(file, "utf8"));
} catch (e) {
  console.error(`could not read ${file}: ${e.message}`);
  process.exit(1);
}

const problems = [];

if (/portable/i.test(JSON.stringify(manifest))) {
  problems.push("it names a portable asset, which the installed updater cannot apply");
}

const platforms = manifest.platforms ?? {};
if (Object.keys(platforms).length === 0) {
  problems.push("it names no platforms at all");
}

/** What the installed updater can actually download and apply. */
const APPLICABLE = /-setup\.exe$|\.msi$|\.app\.tar\.gz$/i;

for (const [key, entry] of Object.entries(platforms)) {
  const url = decodeURIComponent(entry?.url ?? "");
  if (!APPLICABLE.test(url)) {
    problems.push(`platform ${key} points at ${url}, which is not an installed updater artifact`);
  }
  if (!entry?.signature) {
    problems.push(`platform ${key} has no signature`);
  }
  // An architecture pointing at the other architecture's installer is an update
  // that downloads, verifies, installs, and leaves the machine broken.
  if (key === "windows-x86_64" && /arm64|aarch64/i.test(url)) {
    problems.push(`platform ${key} points at an ARM64 artifact: ${url}`);
  }
  if (key === "windows-aarch64" && /(^|[^a-z])x64|x86_64/i.test(url) && !/arm64|aarch64/i.test(url)) {
    problems.push(`platform ${key} points at an x64 artifact: ${url}`);
  }
}

console.log(`${file}`);
console.log(`  version   ${manifest.version}`);
console.log(`  platforms ${Object.keys(platforms).sort().join(", ")}`);
for (const [key, entry] of Object.entries(platforms)) {
  console.log(`    ${key} -> ${decodeURIComponent(entry?.url ?? "")}`);
}

if (problems.length > 0) {
  console.error("");
  for (const problem of problems) console.error(`  FAIL  ${problem}`);
  process.exit(1);
}
console.log("\nEvery expectation held.");
