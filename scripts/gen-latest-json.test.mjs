import { describe, expect, it } from "vitest";
import { buildPlatforms, platformsFor } from "./gen-latest-json.mjs";

/**
 * The updater manifest is the one file in the release that an installed ArcScan
 * downloads and acts on without a person looking at it. Everything it names
 * gets handed to the NSIS updater, so what it must never name is a portable
 * ZIP -- and "it happens not to match the current suffix list" is not a
 * guarantee, it is a coincidence waiting to be broken by an asset rename.
 */

const V = "v1.8.4";
const REPO = "kingnazz/ArcScan";

/** Every asset a 1.8.4 release publishes, plus their signatures. */
const RELEASE = [
  "ArcScan_1.8.4_x64-setup.exe",
  "ArcScan_1.8.4_x64-setup.exe.sig",
  "ArcScan_1.8.4_arm64-setup.exe",
  "ArcScan_1.8.4_arm64-setup.exe.sig",
  "ArcScan_1.8.4_universal.dmg",
  "ArcScan_1.8.4_universal.app.tar.gz",
  "ArcScan_1.8.4_universal.app.tar.gz.sig",
  "ArcScan_1.8.4_windows-x64-portable.zip",
  "ArcScan_1.8.4_windows-arm64-portable.zip",
];

const platforms = () => buildPlatforms(RELEASE, () => "SIGNATURE", V, REPO);

describe("platformsFor", () => {
  it("maps the installer artifacts to their platforms", () => {
    expect(platformsFor("ArcScan_1.8.4_x64-setup.exe")).toEqual(["windows-x86_64"]);
    expect(platformsFor("ArcScan_1.8.4_arm64-setup.exe")).toEqual(["windows-aarch64"]);
    expect(platformsFor("ArcScan_1.8.4_universal.app.tar.gz")).toEqual([
      "darwin-x86_64",
      "darwin-aarch64",
    ]);
  });

  it("maps no portable asset to any platform", () => {
    for (const name of [
      "ArcScan_1.8.4_windows-x64-portable.zip",
      "ArcScan_1.8.4_windows-arm64-portable.zip",
      // Names this release does not use, and names a later one might. All
      // refused, because the rule is the word, not the current suffix.
      "ArcScan_1.8.4_portable_x64.nsis.zip",
      "ArcScan-portable-1.8.4-x64-setup.exe",
      "ArcScan_portable_1.8.4.msi",
      "ArcScan_1.9.0_windows-x64-PORTABLE.zip",
    ]) {
      expect(platformsFor(name), name).toEqual([]);
    }
  });

  it("maps nothing else that turns up in a release directory", () => {
    for (const name of ["latest.json", "ArcScan_1.8.4_universal.dmg", "source.zip", "README.md"]) {
      expect(platformsFor(name), name).toEqual([]);
    }
  });
});

describe("the manifest built from a full 1.8.4 release", () => {
  it("names exactly the three installed-updater platforms", () => {
    expect(Object.keys(platforms()).sort()).toEqual([
      "darwin-aarch64",
      "darwin-x86_64",
      "windows-aarch64",
      "windows-x86_64",
    ]);
  });

  it("points each Windows platform at its own architecture's installer", () => {
    const map = platforms();
    expect(map["windows-x86_64"].url).toContain("x64-setup.exe");
    expect(map["windows-x86_64"].url).not.toContain("arm64");
    expect(map["windows-aarch64"].url).toContain("arm64-setup.exe");
    expect(map["windows-aarch64"].url).not.toContain("x64-setup");
  });

  it("mentions no portable ZIP anywhere in it", () => {
    const json = JSON.stringify(platforms());
    expect(json.toLowerCase()).not.toContain("portable");
    expect(json).not.toContain(".zip");
  });

  it("would still refuse a portable ZIP that somehow arrived signed", () => {
    // Defence in depth. If a future workflow bug signed a portable ZIP, the
    // manifest must still not carry it.
    const withSignedPortable = [
      ...RELEASE,
      "ArcScan_1.8.4_windows-x64-portable.zip.sig",
      "ArcScan_1.8.4_windows-arm64-portable.zip.sig",
    ];
    const map = buildPlatforms(withSignedPortable, () => "SIGNATURE", V, REPO);
    expect(JSON.stringify(map).toLowerCase()).not.toContain("portable");
    expect(map["windows-x86_64"].url).toContain("x64-setup.exe");
  });

  it("prefers the NSIS installer over an MSI when both are signed", () => {
    const withMsi = [
      ...RELEASE,
      "ArcScan_1.8.4_x64_en-US.msi",
      "ArcScan_1.8.4_x64_en-US.msi.sig",
    ];
    const map = buildPlatforms(withMsi, () => "SIGNATURE", V, REPO);
    expect(map["windows-x86_64"].url).toContain("x64-setup.exe");
  });
});
