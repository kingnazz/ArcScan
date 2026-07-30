import { beforeEach, describe, expect, it } from "vitest";
import {
  DEFAULT_SETTINGS,
  LEGACY_KNOWN_KEY,
  clearRecentTargets,
  loadRecentTargets,
  loadSettings,
  markLegacyLabelsImported,
  pendingLegacyLabels,
  pushRecentTarget,
  saveSettings,
} from "./prefs";

beforeEach(() => {
  localStorage.clear();
});

describe("settings", () => {
  it("returns the defaults when nothing is stored", () => {
    expect(loadSettings()).toEqual(DEFAULT_SETTINGS);
  });

  it("round-trips a saved change", () => {
    saveSettings({ ...DEFAULT_SETTINGS, density: "comfortable", historyRetention: 42 });
    const loaded = loadSettings();
    expect(loaded.density).toBe("comfortable");
    expect(loaded.historyRetention).toBe(42);
  });

  it("keeps the public-IP lookup off unless it was explicitly enabled", () => {
    expect(DEFAULT_SETTINGS.publicIpLookup).toBe(false);
    // Absent, null and any non-true value all mean off.
    localStorage.setItem("arcscan-settings", JSON.stringify({ publicIpLookup: "yes" }));
    expect(loadSettings().publicIpLookup).toBe(false);
    localStorage.setItem("arcscan-settings", JSON.stringify({ publicIpLookup: true }));
    expect(loadSettings().publicIpLookup).toBe(true);
  });

  it("survives a corrupt or hostile preferences blob", () => {
    localStorage.setItem("arcscan-settings", "{not json");
    expect(loadSettings()).toEqual(DEFAULT_SETTINGS);

    localStorage.setItem("arcscan-settings", JSON.stringify("a string"));
    expect(loadSettings()).toEqual(DEFAULT_SETTINGS);

    localStorage.setItem("arcscan-settings", JSON.stringify(null));
    expect(loadSettings()).toEqual(DEFAULT_SETTINGS);
  });

  it("clamps out-of-range numbers instead of passing them to a scan", () => {
    saveSettings({
      ...DEFAULT_SETTINGS,
      timeoutMs: 9_999_999,
      hostConcurrency: -5,
      tcpConcurrency: 0,
      pingConcurrency: 5_000,
      historyRetention: 1,
    });
    const loaded = loadSettings();
    expect(loaded.timeoutMs).toBe(10_000);
    expect(loaded.hostConcurrency).toBe(1);
    expect(loaded.tcpConcurrency).toBe(8);
    expect(loaded.pingConcurrency).toBe(128);
    expect(loaded.historyRetention).toBe(5);
  });

  it("rejects unknown enum values in favour of the default", () => {
    localStorage.setItem(
      "arcscan-settings",
      JSON.stringify({ theme: "neon", density: "enormous", defaultProfile: "nope", sortKey: "??" }),
    );
    const loaded = loadSettings();
    expect(loaded.theme).toBe(DEFAULT_SETTINGS.theme);
    expect(loaded.density).toBe(DEFAULT_SETTINGS.density);
    expect(loaded.defaultProfile).toBe(DEFAULT_SETTINGS.defaultProfile);
    expect(loaded.sortKey).toBe(DEFAULT_SETTINGS.sortKey);
  });

  it("drops hidden-column entries that are not real columns", () => {
    localStorage.setItem(
      "arcscan-settings",
      JSON.stringify({ hiddenColumns: ["vendor", "made-up", 7] }),
    );
    expect(loadSettings().hiddenColumns).toEqual(["vendor"]);
  });
});

describe("recent targets", () => {
  it("records the newest first and de-duplicates", () => {
    pushRecentTarget("192.168.1.0/24");
    pushRecentTarget("10.0.0.0/24");
    const list = pushRecentTarget("192.168.1.0/24");
    expect(list).toEqual(["192.168.1.0/24", "10.0.0.0/24"]);
  });

  it("caps the list so it never grows without bound", () => {
    for (let n = 1; n <= 20; n++) pushRecentTarget(`10.0.${n}.0/24`);
    expect(loadRecentTargets()).toHaveLength(8);
    expect(loadRecentTargets()[0]).toBe("10.0.20.0/24");
  });

  it("ignores a blank target", () => {
    pushRecentTarget("10.0.0.0/24");
    expect(pushRecentTarget("   ")).toEqual(["10.0.0.0/24"]);
  });

  it("clears on request", () => {
    pushRecentTarget("10.0.0.0/24");
    expect(clearRecentTargets()).toEqual([]);
    expect(loadRecentTargets()).toEqual([]);
  });

  it("recovers from a corrupt list", () => {
    localStorage.setItem("arcscan-recent-targets", JSON.stringify({ nope: true }));
    expect(loadRecentTargets()).toEqual([]);
  });
});

describe("v1.6 device-label migration", () => {
  it("offers the stored labels once, then never again", () => {
    localStorage.setItem(
      LEGACY_KNOWN_KEY,
      JSON.stringify({ "AA:BB:CC:00:00:01": "Reception NAS", "AA:BB:CC:00:00:02": "" }),
    );

    const labels = pendingLegacyLabels();
    expect(labels).toEqual({
      "AA:BB:CC:00:00:01": "Reception NAS",
      "AA:BB:CC:00:00:02": "",
    });

    // Once imported, the labels must not be pushed back over later edits.
    markLegacyLabelsImported();
    expect(pendingLegacyLabels()).toBeNull();
  });

  it("returns null when there is nothing to migrate", () => {
    expect(pendingLegacyLabels()).toBeNull();
    localStorage.setItem(LEGACY_KNOWN_KEY, JSON.stringify({}));
    expect(pendingLegacyLabels()).toBeNull();
  });

  it("ignores entries that are not label strings", () => {
    localStorage.setItem(
      LEGACY_KNOWN_KEY,
      JSON.stringify({ "AA:BB:CC:00:00:01": "Keep", "AA:BB:CC:00:00:02": { nested: true } }),
    );
    expect(pendingLegacyLabels()).toEqual({ "AA:BB:CC:00:00:01": "Keep" });
  });

  it("survives a corrupt legacy blob", () => {
    localStorage.setItem(LEGACY_KNOWN_KEY, "[1,2,3]");
    expect(pendingLegacyLabels()).toBeNull();
  });
});
