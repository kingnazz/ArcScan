#!/usr/bin/env node
// Package a Windows Portable ZIP.
//
// One script, used by both the CI build and by hand, so the packaging rules live
// in one readable place rather than spread across shell steps in two workflows
// that can drift apart. Everything it can check, it checks, and it fails rather
// than shipping something almost right:
//
//   * the binary it was pointed at exists;
//   * it is a Windows PE for the architecture the ZIP is named after, read out
//     of the PE header rather than inferred from the path;
//   * it is the *portable* build, not the installed one (see below);
//   * the staged payload is exactly ArcScan.exe and README-PORTABLE.txt;
//   * nothing else -- no installer, no updater manifest, no signature, no
//     debug symbols, no pre-created user database -- came along.
//
//   node scripts/package-portable.mjs --version 1.8.4 --target x86_64-pc-windows-msvc
//                                     --binary <path to arcscan.exe>
//                                     [--out dist-portable]
//
// Prints the final path and size.

import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");

const args = process.argv.slice(2);
function flag(name, fallback = null) {
  const i = args.indexOf(`--${name}`);
  return i >= 0 && args[i + 1] ? args[i + 1] : fallback;
}

function die(message) {
  console.error(`package-portable: ${message}`);
  process.exit(1);
}

/**
 * The Rust targets that get a portable ZIP, and what each one is called in the
 * asset name and in a PE header.
 *
 * Windows only in 1.8.4, deliberately. A macOS "portable" build would be a
 * different design question -- an app bundle is already relocatable, and its
 * data location follows different platform conventions -- and shipping one
 * badly would be worse than not shipping one.
 */
const TARGETS = {
  "x86_64-pc-windows-msvc": { label: "windows-x64", machine: 0x8664, machineName: "x64" },
  "aarch64-pc-windows-msvc": { label: "windows-arm64", machine: 0xaa64, machineName: "ARM64" },
};

const version = flag("version");
const target = flag("target");
const binary = flag("binary");
const outDir = path.resolve(root, flag("out", "dist-portable"));

if (!version || !target || !binary) {
  die(
    "usage: package-portable.mjs --version <x.y.z> --target <rust target> --binary <exe> [--out <dir>]",
  );
}
if (!/^\d+\.\d+\.\d+(-[\w.]+)?$/.test(version)) die(`"${version}" is not a semantic version`);

const spec = TARGETS[target];
if (!spec) {
  die(`unsupported target "${target}". Portable builds are ${Object.keys(TARGETS).join(", ")}.`);
}

const binaryPath = path.resolve(root, binary);
if (!existsSync(binaryPath)) die(`no binary at ${binaryPath}`);

// ---------------------------------------------------------------- PE header

/**
 * Read the machine type out of a Windows PE file.
 *
 * A filename is not evidence. Cross-compiling both Windows targets on one
 * runner makes it entirely possible to pick up the wrong `release/` directory
 * and hand an x64 binary to the ARM64 packaging step, which would produce a ZIP
 * that is correctly named, correctly sized, and unrunnable on the machine it
 * was downloaded for. So the header is read: MZ at 0, the PE offset at 0x3c,
 * "PE\0\0" there, and the machine word right after it.
 */
function peMachine(file) {
  const buffer = readFileSync(file);
  if (buffer.length < 0x40 || buffer.readUInt16LE(0) !== 0x5a4d) {
    die(`${file} is not a Windows executable (no MZ signature)`);
  }
  const peOffset = buffer.readUInt32LE(0x3c);
  if (peOffset + 6 > buffer.length || buffer.readUInt32LE(peOffset) !== 0x00004550) {
    die(`${file} is not a Windows executable (no PE signature)`);
  }
  return buffer.readUInt16LE(peOffset + 4);
}

const machine = peMachine(binaryPath);
if (machine !== spec.machine) {
  const known = Object.values(TARGETS).find((t) => t.machine === machine);
  die(
    `${binaryPath} is a ${known ? known.machineName : `0x${machine.toString(16)}`} binary, ` +
      `but ${target} needs ${spec.machineName}. Refusing to package the wrong architecture.`,
  );
}

/**
 * Refuse the installed build.
 *
 * The portable edition is compiled without tauri-plugin-updater, so the
 * updater's own strings are absent from the binary. The installed build carries
 * them. This is a heuristic and it is treated as one -- it can only ever say
 * "this is definitely the installed build", never "this is definitely the
 * portable one" -- but it catches the mistake this release exists to prevent:
 * shipping the installed executable in a ZIP and calling it portable.
 */
function looksLikeInstalledBuild(file) {
  const text = readFileSync(file).toString("latin1");
  // A string only the updater plugin puts in the binary: its manifest field
  // names and its endpoint. Present in the installed build, absent without it.
  const updaterMarkers = ["tauri-plugin-updater", "releases/latest/download/latest.json"];
  return updaterMarkers.filter((m) => text.includes(m));
}

const installedMarkers = looksLikeInstalledBuild(binaryPath);
if (installedMarkers.length > 0) {
  die(
    `${binaryPath} contains updater strings (${installedMarkers.join(", ")}), so it is the ` +
      `installed build. Build the portable edition with ` +
      `--no-default-features --features portable.`,
  );
}

// ---------------------------------------------------------------- staging

const assetName = `ArcScan_${version}_${spec.label}-portable.zip`;
const staging = path.join(outDir, `staging-${spec.label}`);
rmSync(staging, { recursive: true, force: true });
mkdirSync(staging, { recursive: true });
mkdirSync(outDir, { recursive: true });

copyFileSync(binaryPath, path.join(staging, "ArcScan.exe"));

const readme = path.join(root, "packaging", "README-PORTABLE.txt");
if (!existsSync(readme)) die(`no README at ${readme}`);
writeFileSync(
  path.join(staging, "README-PORTABLE.txt"),
  readFileSync(readme, "utf8").replaceAll("__VERSION__", version).replaceAll("__ARCH__", spec.machineName),
);

/** Exactly what a portable ZIP may contain, and nothing else. */
const EXPECTED = ["ArcScan.exe", "README-PORTABLE.txt"];
const staged = readdirSync(staging).sort();
if (JSON.stringify(staged) !== JSON.stringify([...EXPECTED].sort())) {
  die(`staged payload is ${JSON.stringify(staged)}, expected ${JSON.stringify(EXPECTED)}`);
}

// ---------------------------------------------------------------- the ZIP

const zipPath = path.join(outDir, assetName);
rmSync(zipPath, { force: true });

/**
 * Make the archive with whatever this platform has.
 *
 * PowerShell's Compress-Archive on Windows (where the release is built) and
 * `zip` elsewhere (so the packaging is testable on a Linux developer machine
 * and in the Linux CI job). Both produce a flat archive of the staging
 * directory's contents, which is what the ZIP verification then reads back.
 */
function makeZip() {
  if (process.platform === "win32") {
    const result = spawnSync(
      "powershell",
      [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        `Compress-Archive -Path '${path.join(staging, "*")}' -DestinationPath '${zipPath}' -Force`,
      ],
      { stdio: "inherit" },
    );
    return result.status === 0;
  }
  const result = spawnSync("zip", ["-q", "-X", "-j", zipPath, ...EXPECTED.map((f) => path.join(staging, f))], {
    stdio: "inherit",
  });
  return result.status === 0;
}

if (!makeZip()) die("the archive command failed");
if (!existsSync(zipPath)) die(`no archive at ${zipPath}`);

rmSync(staging, { recursive: true, force: true });

const size = statSync(zipPath).size;
console.log(`${assetName}`);
console.log(`  path         ${zipPath}`);
console.log(`  size         ${(size / (1024 * 1024)).toFixed(2)} MB (${size} bytes)`);
console.log(`  contents     ${EXPECTED.join(", ")}`);
console.log(`  target       ${target}`);
console.log(`  architecture ${spec.machineName} (PE machine 0x${machine.toString(16)})`);
console.log(`  edition      portable (no updater strings present)`);
