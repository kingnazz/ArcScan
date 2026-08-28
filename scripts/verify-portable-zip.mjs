#!/usr/bin/env node
// Verify a packaged portable ZIP, from the outside.
//
// package-portable.mjs checks what it is about to put in the archive. This
// checks what came out of it, which is not the same thing: it opens the ZIP that
// will actually be uploaded to a GitHub release, lists what is in it, and reads
// the PE header of the executable it contains.
//
// The distinction matters because these two scripts fail differently. A bug in
// the packaging script's staging would pass its own checks and produce a wrong
// archive; a bug in the archiving step -- a stale file picked up, a directory
// flattened wrongly, an old ZIP not replaced -- would not be visible from the
// staging directory at all.
//
//   node scripts/verify-portable-zip.mjs --zip <path> --architecture x64|ARM64
//                                        --version 1.8.4
//
// Exits non-zero on the first failed expectation.

import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

const args = process.argv.slice(2);
const flag = (name) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? args[i + 1] : null;
};

const zip = flag("zip");
const architecture = flag("architecture");
const version = flag("version");

if (!zip || !architecture || !version) {
  console.error(
    "usage: verify-portable-zip.mjs --zip <path> --architecture x64|ARM64 --version <x.y.z>",
  );
  process.exit(1);
}

const MACHINES = { x64: 0x8664, ARM64: 0xaa64 };
if (!(architecture in MACHINES)) {
  console.error(`unknown architecture "${architecture}" (expected ${Object.keys(MACHINES).join(" or ")})`);
  process.exit(1);
}

let failures = 0;
function check(ok, description, detail) {
  if (ok) console.log(`  ok    ${description}`);
  else {
    failures += 1;
    console.log(`  FAIL  ${description}${detail ? `\n        ${detail}` : ""}`);
  }
}

if (!existsSync(zip)) {
  console.error(`no archive at ${zip}`);
  process.exit(1);
}

const work = mkdtempSync(path.join(tmpdir(), "arcscan-zip-verify-"));

/** Extract with whatever this platform has: PowerShell on Windows, unzip elsewhere. */
function extract() {
  if (process.platform === "win32") {
    return spawnSync(
      "powershell",
      [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        `Expand-Archive -Path '${path.resolve(zip)}' -DestinationPath '${work}' -Force`,
      ],
      { stdio: "inherit" },
    ).status === 0;
  }
  return spawnSync("unzip", ["-q", path.resolve(zip), "-d", work], { stdio: "inherit" }).status === 0;
}

if (!extract()) {
  console.error("could not extract the archive");
  process.exit(1);
}

/** Every path inside the extracted tree, relative and slash-separated. */
function walk(dir, prefix = "") {
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) out.push(`${rel}/`, ...walk(path.join(dir, entry.name), rel));
    else out.push(rel);
  }
  return out.sort();
}

const contents = walk(work);
const size = statSync(zip).size;

console.log(`${path.basename(zip)}`);
console.log(`  size      ${(size / (1024 * 1024)).toFixed(2)} MB (${size} bytes)`);
console.log(`  contents  ${JSON.stringify(contents)}`);
console.log("");

// -------------------------------------------------------------- the payload

const EXPECTED = ["ArcScan.exe", "README-PORTABLE.txt"];
check(
  JSON.stringify(contents) === JSON.stringify([...EXPECTED].sort()),
  "it contains exactly ArcScan.exe and README-PORTABLE.txt",
  `found ${JSON.stringify(contents)}`,
);

/**
 * Things that must never be in a portable ZIP, and why each one matters.
 *
 * An installer or a DMG would mean the wrong artifact was collected. An updater
 * manifest or a signature would suggest the portable ZIP is an updater payload,
 * which it must never be. Debug symbols and sources are not something to ship
 * to users. And a pre-created database or ArcScanData folder would mean every
 * download shared one starting state -- and would put somebody's test data in
 * everybody's otherwise fresh temporary session.
 */
const FORBIDDEN = [
  [/\.msi$/i, "MSI installer"],
  [/-setup\.exe$/i, "installer executable"],
  [/\.dmg$/i, "macOS disk image"],
  [/\.app(\/|$)/i, "macOS app bundle"],
  [/latest\.json$/i, "updater manifest"],
  [/\.sig$/i, "updater signature"],
  [/\.pdb$/i, "debug symbols"],
  [/\.(rs|ts|tsx)$/i, "source files"],
  [/\.nsis\.zip$/i, "NSIS updater archive"],
  [/^arcscan\.db/i, "pre-created user database"],
  [/ArcScanData/i, "pre-created ArcScanData folder"],
  [/MicrosoftEdgeWebview2Setup|WebView2.*\.exe/i, "WebView2 bootstrapper"],
  [/\.test\./i, "test artifacts"],
];

for (const [pattern, description] of FORBIDDEN) {
  const hits = contents.filter((name) => pattern.test(name));
  check(hits.length === 0, `it carries no ${description}`, hits.join(", "));
}

// ---------------------------------------------------------- the executable

const exe = path.join(work, "ArcScan.exe");
check(existsSync(exe), "ArcScan.exe is there");

if (existsSync(exe)) {
  const buffer = readFileSync(exe);
  const mz = buffer.length > 0x40 && buffer.readUInt16LE(0) === 0x5a4d;
  check(mz, "it is a Windows executable");
  if (mz) {
    const peOffset = buffer.readUInt32LE(0x3c);
    const pe = buffer.readUInt32LE(peOffset) === 0x00004550;
    check(pe, "it has a PE header");
    if (pe) {
      const machine = buffer.readUInt16LE(peOffset + 4);
      check(
        machine === MACHINES[architecture],
        `its PE machine type is ${architecture}`,
        `header says 0x${machine.toString(16)}, expected 0x${MACHINES[architecture].toString(16)}`,
      );
    }
  }

  // The release's whole premise: this is the portable build, not the installed
  // executable in a ZIP. Each of these strings is in the binary only because
  // the updater plugin is linked, verified against both editions built from
  // this tree. The updater *endpoint* is deliberately not among them:
  // generate_context! embeds the whole of tauri.conf.json, so that URL is in
  // the portable binary too.
  const text = buffer.toString("latin1");
  for (const marker of [
    "tauri-plugin-updater",
    "plugin:updater",
    "download_and_install",
    "minisign",
  ]) {
    check(!text.includes(marker), `it is the portable build (no "${marker}")`);
  }

  // And it is this version's build.
  check(
    text.includes(version),
    `it carries the version string ${version}`,
    "the version was not found anywhere in the executable",
  );
}

// -------------------------------------------------------------- the README

const readme = path.join(work, "README-PORTABLE.txt");
if (existsSync(readme)) {
  const text = readFileSync(readme, "utf8");
  check(text.includes(`ArcScan ${version}`), `the README names version ${version}`);
  check(text.includes(architecture), `the README names ${architecture}`);
  check(!text.includes("__VERSION__") && !text.includes("__ARCH__"), "no placeholder is left in it");
  check(
    /Every Portable process creates its own private session/i.test(text),
    "it states that each launch uses a fresh temporary session",
  );
  check(
    /next (?:Portable )?launch starts fresh/i.test(text),
    "it states that the next Portable launch starts fresh",
  );
  check(/CSV, JSON (?:and|or) XML exports/i.test(text), "it names every intentional export format");
  check(
    /export.{0,40}anything you want to keep/i.test(text),
    "it tells the operator to export anything they want to retain",
  );
  check(/read-only/i.test(text), "it permits an extracted read-only executable folder");
  check(
    !/ArcScan-owned persistent data|ArcScanData appears/i.test(text),
    "it carries no obsolete persistent-folder instructions",
  );
  check(text.includes("WebView2"), "it states the WebView2 requirement");
  check(
    !/zero dependencies/i.test(text),
    "it does not claim zero dependencies",
  );
  check(
    !/writes nothing (?:else )?outside/i.test(text),
    "it does not claim Windows writes nothing outside the folder",
  );
}

rmSync(work, { recursive: true, force: true });

console.log("");
if (failures > 0) {
  console.error(`${failures} expectation(s) failed.`);
  process.exit(1);
}
console.log("Every expectation held.");
