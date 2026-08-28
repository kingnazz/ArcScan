import { describe, expect, it } from "vitest";
import { PORTABLE_UPDATE_STEPS, editionLabel, isPortable, type RuntimeInfo } from "./runtime";

const installed: RuntimeInfo = {
  edition: "installed",
  version: "1.8.4",
  platform: "Windows",
  architecture: "x64",
  storage_mode: "persistent",
  data_root: "C:\\Users\\Operator\\AppData\\Roaming\\com.arcscan.app",
  updater_mode: "installer",
};

const portable: RuntimeInfo = {
  edition: "portable",
  version: "1.8.4",
  platform: "Windows",
  architecture: "ARM64",
  storage_mode: "temporary",
  data_root: null,
  updater_mode: "manual",
};

describe("isPortable", () => {
  it("is false while the backend has not answered", () => {
    // The safe reading in both directions: an installed build is what every
    // release before 1.8.4 was, and a portable build has no installer updater
    // to re-enable.
    expect(isPortable(null)).toBe(false);
  });

  it("follows the edition the backend reported", () => {
    expect(isPortable(installed)).toBe(false);
    expect(isPortable(portable)).toBe(true);
  });
});

describe("editionLabel", () => {
  it("names the edition and the build's own architecture", () => {
    expect(editionLabel(portable)).toBe("Portable edition · Windows ARM64");
    expect(editionLabel(installed)).toBe("Installed edition · Windows x64");
  });

  it("says ARM64 for an ARM64 build whatever it is running on", () => {
    // The whole reason the architecture comes from the build rather than the
    // user agent: an x64 build on an ARM64 machine must not claim to be native.
    expect(editionLabel({ ...portable, architecture: "x64" })).toContain("x64");
    expect(editionLabel({ ...portable, architecture: "ARM64" })).toContain("ARM64");
  });
});

describe("the portable update wording", () => {
  it("says what to do and what to keep, and never offers to install", () => {
    expect(PORTABLE_UPDATE_STEPS).toContain("Portable ZIP");
    expect(PORTABLE_UPDATE_STEPS).toContain("Export anything");
    expect(PORTABLE_UPDATE_STEPS).toContain("finish");
    expect(PORTABLE_UPDATE_STEPS).toContain("close ArcScan");
    expect(PORTABLE_UPDATE_STEPS.toLowerCase()).not.toContain("update now");
    expect(PORTABLE_UPDATE_STEPS.toLowerCase()).not.toContain("install");
    expect(PORTABLE_UPDATE_STEPS).not.toContain("ArcScanData");
  });
});
