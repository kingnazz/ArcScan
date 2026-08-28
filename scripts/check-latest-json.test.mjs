import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";

const work = [];

afterEach(() => {
  for (const directory of work.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
});

function manifest(overrides = {}) {
  const version = overrides.version ?? "1.8.4";
  const tag = overrides.tag ?? `v${version}`;
  const base = `https://github.com/kingnazz/ArcScan/releases/download/${tag}`;
  const platforms = {
    "darwin-aarch64": {
      signature: "MAC-SIGNATURE",
      url: `${base}/ArcScan.app.tar.gz`,
    },
    "darwin-x86_64": {
      signature: "MAC-SIGNATURE",
      url: `${base}/ArcScan.app.tar.gz`,
    },
    "windows-aarch64": {
      signature: "ARM-SIGNATURE",
      url: `${base}/ArcScan_${version}_arm64-setup.exe`,
    },
    "windows-x86_64": {
      signature: "X64-SIGNATURE",
      url: `${base}/ArcScan_${version}_x64-setup.exe`,
    },
    ...(overrides.platforms ?? {}),
  };
  for (const key of overrides.removePlatforms ?? []) delete platforms[key];
  return { version, notes: `ArcScan ${tag}`, pub_date: "2026-08-27T00:00:00Z", platforms };
}

function run(value, expected = "v1.8.4") {
  const directory = mkdtempSync(path.join(tmpdir(), "arcscan-latest-check-"));
  work.push(directory);
  const file = path.join(directory, "latest.json");
  writeFileSync(file, JSON.stringify(value));
  return spawnSync(process.execPath, ["scripts/check-latest-json.mjs", file, expected], {
    cwd: path.resolve(import.meta.dirname, ".."),
    encoding: "utf8",
  });
}

describe("the publication-time latest.json check", () => {
  it("accepts the exact Installed updater manifest published for 1.8.4", () => {
    const result = run(manifest());
    expect(result.status, result.stderr).toBe(0);
    expect(result.stdout).toContain("Every expectation held");
  });

  it("rejects a Portable ZIP even when it has a signature", () => {
    const value = manifest();
    value.platforms["windows-x86_64"] = {
      signature: "WRONG-SIGNATURE",
      url: "https://github.com/kingnazz/ArcScan/releases/download/v1.8.4/ArcScan_1.8.4_windows-x64-portable.zip",
    };
    const result = run(value);
    expect(result.status).not.toBe(0);
    expect(result.stderr).toMatch(/Portable asset|expected ArcScan_1\.8\.4_x64-setup\.exe/);
  });

  it("rejects stale versions, tags, missing platforms and empty signatures", () => {
    const value = manifest({
      version: "1.8.3",
      tag: "v1.8.3",
      removePlatforms: ["darwin-aarch64"],
    });
    value.platforms["windows-x86_64"].signature = "";
    const result = run(value, "v1.8.4");
    expect(result.status).not.toBe(0);
    expect(result.stderr).toContain("manifest version");
    expect(result.stderr).toContain("platform keys");
    expect(result.stderr).toContain("no non-empty updater signature");
    expect(result.stderr).toContain("does not target release v1.8.4");
  });
});
