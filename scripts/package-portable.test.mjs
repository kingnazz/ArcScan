import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

/**
 * Tests for the thing that decides what actually ships.
 *
 * The failure this guards against is specific and has happened to other
 * projects: both Windows targets cross-compile on one runner, so picking up the
 * wrong `release/` directory hands an x64 binary to the ARM64 packaging step and
 * produces a ZIP that is correctly named, correctly sized, and unrunnable on the
 * machine somebody downloaded it for. Nothing about that is visible without
 * reading the PE header, so the script reads it and these tests check that it
 * does.
 */

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const script = path.join(root, "scripts", "package-portable.mjs");

let work;

beforeEach(() => {
  work = mkdtempSync(path.join(tmpdir(), "arcscan-pkg-"));
});

afterEach(() => {
  rmSync(work, { recursive: true, force: true });
});

/**
 * Write a file with a valid PE header for `machine`, plus some payload.
 *
 * Enough of a PE for the reader under test: MZ at 0, the PE header offset at
 * 0x3c, the PE signature there, and the machine word after it. Not a runnable
 * program, which is fine -- nothing here runs it.
 */
function fakePe(name, machine, { extraStrings = [] } = {}) {
  const file = path.join(work, name);
  const peOffset = 0x80;
  const buffer = Buffer.alloc(0x200);
  buffer.writeUInt16LE(0x5a4d, 0); // "MZ"
  buffer.writeUInt32LE(peOffset, 0x3c);
  buffer.writeUInt32LE(0x00004550, peOffset); // "PE\0\0"
  buffer.writeUInt16LE(machine, peOffset + 4);
  let cursor = peOffset + 0x40;
  for (const text of extraStrings) {
    buffer.write(text, cursor, "latin1");
    cursor += text.length + 1;
  }
  writeFileSync(file, buffer);
  return file;
}

const X64 = 0x8664;
const ARM64 = 0xaa64;

function pack(args) {
  return execFileSync(process.execPath, [script, ...args], { encoding: "utf8" });
}

function packExpectingFailure(args) {
  try {
    execFileSync(process.execPath, [script, ...args], { encoding: "utf8", stdio: "pipe" });
  } catch (e) {
    return `${e.stdout ?? ""}${e.stderr ?? ""}`;
  }
  throw new Error("expected the packaging script to fail, but it succeeded");
}

function contentsOf(zip) {
  return execFileSync("unzip", ["-Z1", zip], { encoding: "utf8" })
    .split("\n")
    .filter(Boolean)
    .sort();
}

describe("packaging a portable ZIP", () => {
  it("produces the expected filename and exactly the expected payload", () => {
    const exe = fakePe("arcscan.exe", X64);
    const out = pack([
      "--version",
      "1.8.4",
      "--target",
      "x86_64-pc-windows-msvc",
      "--binary",
      exe,
      "--out",
      work,
    ]);

    const zip = path.join(work, "ArcScan_1.8.4_windows-x64-portable.zip");
    expect(existsSync(zip)).toBe(true);
    expect(out).toContain("ArcScan_1.8.4_windows-x64-portable.zip");
    expect(contentsOf(zip)).toEqual(["ArcScan.exe", "README-PORTABLE.txt"]);
  });

  it("names the ARM64 ZIP for ARM64 and puts the ARM64 binary in it", () => {
    const exe = fakePe("arcscan.exe", ARM64);
    pack([
      "--version",
      "1.8.4",
      "--target",
      "aarch64-pc-windows-msvc",
      "--binary",
      exe,
      "--out",
      work,
    ]);
    const zip = path.join(work, "ArcScan_1.8.4_windows-arm64-portable.zip");
    expect(existsSync(zip)).toBe(true);
    expect(contentsOf(zip)).toEqual(["ArcScan.exe", "README-PORTABLE.txt"]);
  });

  it("carries no installer, no updater manifest, no signature and no database", () => {
    const exe = fakePe("arcscan.exe", X64);
    pack([
      "--version",
      "1.8.4",
      "--target",
      "x86_64-pc-windows-msvc",
      "--binary",
      exe,
      "--out",
      work,
    ]);
    const names = contentsOf(path.join(work, "ArcScan_1.8.4_windows-x64-portable.zip"));
    for (const forbidden of [
      /\.msi$/i,
      /-setup\.exe$/i,
      /\.dmg$/i,
      /latest\.json$/i,
      /\.sig$/i,
      /\.pdb$/i,
      /\.rs$/i,
      /arcscan\.db$/i,
      /ArcScanData/i,
    ]) {
      expect(names.filter((n) => forbidden.test(n))).toEqual([]);
    }
  });

  it("fills the version and the architecture into the README", () => {
    const exe = fakePe("arcscan.exe", ARM64);
    pack([
      "--version",
      "1.8.4",
      "--target",
      "aarch64-pc-windows-msvc",
      "--binary",
      exe,
      "--out",
      work,
    ]);
    const extracted = path.join(work, "unzipped");
    mkdirSync(extracted, { recursive: true });
    execFileSync("unzip", [
      "-q",
      path.join(work, "ArcScan_1.8.4_windows-arm64-portable.zip"),
      "-d",
      extracted,
    ]);
    const readme = readFileSync(path.join(extracted, "README-PORTABLE.txt"), "utf8");
    expect(readme).toContain("ArcScan 1.8.4 - Portable Edition for Windows ARM64");
    expect(readme).not.toContain("__VERSION__");
    expect(readme).not.toContain("__ARCH__");
    // The claims the release must not overstate.
    expect(readme).toContain("ArcScan-owned persistent data stays in ArcScanData");
    expect(readme).toContain("WebView2");
    expect(readme.toLowerCase()).not.toContain("zero dependencies");
  });
});

describe("refusing to package the wrong thing", () => {
  it("refuses an x64 binary for the ARM64 ZIP", () => {
    const exe = fakePe("arcscan.exe", X64);
    const output = packExpectingFailure([
      "--version",
      "1.8.4",
      "--target",
      "aarch64-pc-windows-msvc",
      "--binary",
      exe,
      "--out",
      work,
    ]);
    expect(output).toMatch(/is a x64 binary, but aarch64-pc-windows-msvc needs ARM64/);
    expect(existsSync(path.join(work, "ArcScan_1.8.4_windows-arm64-portable.zip"))).toBe(false);
  });

  it("refuses an ARM64 binary for the x64 ZIP", () => {
    const exe = fakePe("arcscan.exe", ARM64);
    const output = packExpectingFailure([
      "--version",
      "1.8.4",
      "--target",
      "x86_64-pc-windows-msvc",
      "--binary",
      exe,
      "--out",
      work,
    ]);
    expect(output).toMatch(/is a ARM64 binary, but x86_64-pc-windows-msvc needs x64/);
  });

  it("refuses the installed build, which is the whole point of the release", () => {
    // A binary that still carries the updater plugin's strings is the installed
    // edition, and a ZIP containing it would be exactly the "installed exe in a
    // ZIP" this release exists not to ship.
    const exe = fakePe("arcscan.exe", X64, {
      extraStrings: ["tauri-plugin-updater", "releases/latest/download/latest.json"],
    });
    const output = packExpectingFailure([
      "--version",
      "1.8.4",
      "--target",
      "x86_64-pc-windows-msvc",
      "--binary",
      exe,
      "--out",
      work,
    ]);
    expect(output).toMatch(/contains updater strings/);
    expect(output).toMatch(/--no-default-features --features portable/);
  });

  it("refuses something that is not a Windows executable at all", () => {
    const notPe = path.join(work, "arcscan.exe");
    writeFileSync(notPe, "#!/bin/sh\necho hello\n");
    const output = packExpectingFailure([
      "--version",
      "1.8.4",
      "--target",
      "x86_64-pc-windows-msvc",
      "--binary",
      notPe,
      "--out",
      work,
    ]);
    expect(output).toMatch(/not a Windows executable/);
  });

  it("refuses a missing binary", () => {
    const output = packExpectingFailure([
      "--version",
      "1.8.4",
      "--target",
      "x86_64-pc-windows-msvc",
      "--binary",
      path.join(work, "absent.exe"),
      "--out",
      work,
    ]);
    expect(output).toMatch(/no binary at/);
  });

  it("refuses a target with no portable build, rather than inventing one", () => {
    const exe = fakePe("arcscan.exe", X64);
    // macOS portable is explicitly not part of 1.8.4. Asking for it must fail
    // loudly rather than produce something the website could then link to.
    const output = packExpectingFailure([
      "--version",
      "1.8.4",
      "--target",
      "universal-apple-darwin",
      "--binary",
      exe,
      "--out",
      work,
    ]);
    expect(output).toMatch(/unsupported target/);
  });

  it("refuses a version that is not a version", () => {
    const exe = fakePe("arcscan.exe", X64);
    const output = packExpectingFailure([
      "--version",
      "latest",
      "--target",
      "x86_64-pc-windows-msvc",
      "--binary",
      exe,
      "--out",
      work,
    ]);
    expect(output).toMatch(/not a semantic version/);
  });
});
