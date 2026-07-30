#!/usr/bin/env node
// Propagate the version in package.json to every file that has to carry a copy.
//
// package.json is the single source of truth. The UI reads it through a Vite
// define, and everything below is rewritten from it rather than edited by hand,
// because a release where the installer, the in-app version and the website
// disagree is worse than one that is simply late.
//
//   node scripts/sync-version.mjs           rewrite the files
//   node scripts/sync-version.mjs --check   report drift and exit non-zero
//
// CI runs the --check form, so a version bump that misses a file fails the build.

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const check = process.argv.includes("--check");

const version = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
if (!/^\d+\.\d+\.\d+(-[\w.]+)?$/.test(version)) {
  console.error(`package.json version "${version}" is not a semantic version.`);
  process.exit(1);
}

/**
 * Each target names the file, a pattern that matches exactly the version
 * occurrence, and how to rebuild that occurrence. Anchoring on surrounding
 * context rather than on the bare number keeps the rewrite from touching an
 * unrelated version, such as a dependency's.
 */
const targets = [
  {
    file: "src-tauri/Cargo.toml",
    // The [package] version is the first `version = "..."` in the file.
    pattern: /^(version\s*=\s*")[^"]+(")/m,
    replace: (m, a, b) => `${a}${version}${b}`,
  },
  {
    file: "src-tauri/tauri.conf.json",
    pattern: /("version"\s*:\s*")[^"]+(")/,
    replace: (m, a, b) => `${a}${version}${b}`,
  },
  {
    file: "src-tauri/updater.conf.json",
    pattern: /("version"\s*:\s*")[^"]+(")/,
    replace: (m, a, b) => `${a}${version}${b}`,
    optional: true,
  },
  {
    file: "site/index.html",
    pattern: /("softwareVersion"\s*:\s*")[^"]+(")/,
    replace: (m, a, b) => `${a}${version}${b}`,
  },
  {
    file: "site/index.html",
    // The visible version in the hero, rendered before the GitHub API answers.
    pattern: /(<span id="version-fallback">v)[^<]+(<\/span>)/,
    replace: (m, a, b) => `${a}${version}${b}`,
  },
  {
    file: "site/privacy.html",
    pattern: /(<span id="privacy-version">v)[^<]+(<\/span>)/,
    replace: (m, a, b) => `${a}${version}${b}`,
    optional: true,
  },
];

const problems = [];
const updated = new Set();

for (const target of targets) {
  const path = join(root, target.file);
  let text;
  try {
    text = readFileSync(path, "utf8");
  } catch {
    if (!target.optional) problems.push(`${target.file}: file not found`);
    continue;
  }

  const match = text.match(target.pattern);
  if (!match) {
    if (!target.optional) {
      problems.push(`${target.file}: no version placeholder matched ${target.pattern}`);
    }
    continue;
  }

  const next = text.replace(target.pattern, target.replace);
  if (next === text) continue;

  if (check) {
    problems.push(`${target.file}: version is out of date (expected ${version})`);
  } else {
    writeFileSync(path, next);
    updated.add(target.file);
  }
}

if (problems.length > 0) {
  console.error(`Version ${version} is not applied everywhere:`);
  for (const problem of problems) console.error(`  ${problem}`);
  if (check) console.error("\nRun `npm run sync-version` and commit the result.");
  process.exit(1);
}

if (check) {
  console.log(`Version ${version} is consistent across every file.`);
} else if (updated.size === 0) {
  console.log(`Version ${version} was already applied everywhere.`);
} else {
  console.log(`Version ${version} written to:`);
  for (const file of updated) console.log(`  ${file}`);
}
