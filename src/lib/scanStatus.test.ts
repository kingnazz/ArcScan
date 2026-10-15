import { describe, expect, it } from "vitest";
import {
  comparisonPhrase,
  localNetworkFor,
  showsPreflight,
  stageOf,
  statusLine,
  type StatusInput,
} from "./scanStatus";
import { comparisonCount } from "../components/ResultsToolbar";
import type { DeviceDiff, LocalNetwork, ScanComparison } from "../types";
import type { ScanMeta } from "../hooks/useLiveScan";

const WIFI: LocalNetwork = {
  interface: "Wi-Fi",
  ip: "192.168.1.27",
  prefix: 24,
  cidr: "192.168.1.0/24",
  is_private: true,
};

function meta(overrides: Partial<ScanMeta> = {}): ScanMeta {
  return {
    target: "192.168.1.0/24",
    profile: "quick-lan",
    scanId: 7,
    savedScanId: 7,
    durationMs: 2900,
    scanned: 254,
    probed: 254,
    cancelled: false,
    ...overrides,
  };
}

function input(overrides: Partial<StatusInput> = {}): StatusInput {
  return {
    mode: "idle",
    stopping: false,
    progress: null,
    meta: null,
    comparison: null,
    deviceCount: 0,
    target: "192.168.1.0/24",
    preview: { total: 254, port_count: 14, workload: 3556, warning: null },
    localNetworks: [WIFI],
    ...overrides,
  };
}

function diff(kind: DeviceDiff["kind"]): DeviceDiff {
  return { kind, ip: "192.168.1.9", name: "x", device_id: 1, mac: null, vendor: null, hostname: null, last_seen: null, fields: [] };
}

function comparison(overrides: Partial<ScanComparison> = {}): ScanComparison {
  return {
    scan_id: 7,
    baseline_scan_id: 6,
    baseline_created_at: null,
    baseline_target: "192.168.1.0/24",
    reason: null,
    added: [],
    removed: [],
    changed: [],
    ...overrides,
  };
}

describe("scan stages", () => {
  it("folds the backend's six phases into three a person can follow", () => {
    expect(stageOf("probing")).toBe(1);
    expect(stageOf("confirming")).toBe(1);
    expect(stageOf("discovering")).toBe(2);
    expect(stageOf("describing")).toBe(2);
    expect(stageOf("classifying")).toBe(2);
    expect(stageOf("resolving")).toBe(3);
    expect(stageOf(null)).toBe(0);
    expect(stageOf("something-new")).toBe(0);
  });
});

describe("the status line before a scan", () => {
  it("names this computer's network and the workload", () => {
    const line = statusLine(input());
    expect(line.headline).toBe("Local network");
    expect(line.details).toEqual(["Wi-Fi · this computer is 192.168.1.27", "254 addresses", "14 ports"]);
    expect(line.note).toBeNull();
  });

  it("says a routed target cannot show MAC addresses", () => {
    const line = statusLine(input({ target: "10.9.9.0/24" }));
    expect(line.headline).toBe("Routed or custom target");
    expect(line.icon).toBe("routed");
    expect(line.note?.text).toMatch(/MAC addresses/);
  });

  it("asks for a target when the field is empty", () => {
    const line = statusLine(input({ target: "  ", preview: null }));
    expect(line.headline).toBe("Ready");
  });

  it("warns before a credentialed scan that has no credential", () => {
    const line = statusLine(input({ credentialMissing: true }));
    expect(line.note).toEqual(expect.objectContaining({ tone: "warning" }));
  });

  it("passes the backend's workload warning through", () => {
    const line = statusLine(
      input({ preview: { total: 16_777_214, port_count: 14, workload: 234_880_996, warning: "Large" } }),
    );
    expect(line.note).toEqual({ text: "Large", tone: "warning" });
  });

  it("returns to the pre-flight summary once a different target is typed", () => {
    expect(showsPreflight({ mode: "finished", meta: meta(), target: "192.168.1.0/24" })).toBe(false);
    expect(showsPreflight({ mode: "finished", meta: meta(), target: "10.0.0.0/24" })).toBe(true);
    expect(showsPreflight({ mode: "scanning", meta: meta(), target: "10.0.0.0/24" })).toBe(false);
  });

  it("recognises blocks, single hosts and ranges contained by the detected network", () => {
    expect(localNetworkFor(" 192.168.1.0/24 ", [WIFI])).toBe(WIFI);
    expect(localNetworkFor("192.168.1.50", [WIFI])).toBe(WIFI);
    expect(localNetworkFor("192.168.1.1-100", [WIFI])).toBe(WIFI);
    expect(localNetworkFor("192.168.1.1-192.168.1.100", [WIFI])).toBe(WIFI);
  });

  it("does not confuse routed, public or malformed targets with the local network", () => {
    expect(localNetworkFor("192.168.2.50", [WIFI])).toBeNull();
    expect(localNetworkFor("10.20.30.0/24", [WIFI])).toBeNull();
    expect(localNetworkFor("8.8.8.8", [WIFI])).toBeNull();
    expect(localNetworkFor("192.168.1.", [WIFI])).toBeNull();
    expect(localNetworkFor("192.168.1.250-300", [WIFI])).toBeNull();
  });
});

describe("the status line during a scan", () => {
  it("shows the stage and the count of addresses checked while finding hosts", () => {
    const line = statusLine(
      input({
        mode: "scanning",
        meta: meta({ savedScanId: null }),
        deviceCount: 4,
        progress: { scan_id: 7, done: 120, total: 254, found: 5, phase: "probing", elapsed_ms: 900 },
      }),
    );
    expect(line.tone).toBe("live");
    expect(line.headline).toBe("Scanning");
    expect(line.details).toEqual(["192.168.1.0/24", "Step 1 of 3: Finding hosts", "120 of 254 checked", "5 devices found"]);
    expect(line.note?.text).toMatch(/Stop keeps/);
  });

  it("drops the address count once hosts are being identified", () => {
    const line = statusLine(
      input({
        mode: "scanning",
        meta: meta(),
        progress: { scan_id: 7, done: 254, total: 254, found: 12, phase: "classifying", elapsed_ms: 2000 },
      }),
    );
    expect(line.details).toContain("Step 2 of 3: Identifying devices");
    expect(line.details.some((d) => d.includes("checked"))).toBe(false);
  });

  it("says Stop keeps what was found while stopping", () => {
    const line = statusLine(input({ mode: "scanning", stopping: true, meta: meta(), deviceCount: 1 }));
    expect(line.headline).toBe("Stopping");
    expect(line.details).toEqual(["keeping the 1 device found so far"]);
  });
});

describe("the status line after a scan", () => {
  it("summarises a completed scan and its comparison", () => {
    const line = statusLine(
      input({ mode: "finished", meta: meta(), deviceCount: 12, comparison: comparison({ added: [diff("new")] }) }),
    );
    expect(line.tone).toBe("success");
    expect(line.headline).toBe("Scan complete");
    expect(line.details[0]).toBe("12 devices");
    expect(line.comparison).toEqual({ text: "1 new since the previous scan", attention: true });
  });

  it("explains a stopped scan instead of reporting it as complete", () => {
    const line = statusLine(
      input({ mode: "finished", meta: meta({ cancelled: true, probed: 120 }), deviceCount: 4 }),
    );
    expect(line.headline).toBe("Stopped early");
    expect(line.details).toEqual(["4 devices in 120 of 254 addresses"]);
    expect(line.note?.text).toMatch(/never compared/);
    expect(line.comparison).toBeUndefined();
  });

  it("says plainly when nothing answered", () => {
    const line = statusLine(input({ mode: "finished", meta: meta(), deviceCount: 0 }));
    expect(line.headline).toBe("No devices answered");
  });

  it("labels a scan reopened from History as saved, not as just finished", () => {
    const line = statusLine(input({ mode: "history", meta: meta(), deviceCount: 3 }));
    expect(line.headline).toBe("Saved scan");
    expect(line.tone).toBe("neutral");
  });
});

describe("comparison summaries", () => {
  it("does not guess before the comparison exists", () => {
    expect(comparisonPhrase(null)).toBeNull();
  });

  it("says a first scan has nothing to compare with", () => {
    expect(comparisonPhrase(comparison({ baseline_scan_id: null }))?.text).toMatch(/First scan/);
  });

  it("names every kind of difference in a fixed order", () => {
    const phrase = comparisonPhrase(
      comparison({
        added: [diff("new"), diff("returned")],
        changed: [diff("changed"), diff("changed")],
        removed: [diff("missing")],
      }),
    );
    expect(phrase?.text).toBe("1 new, 1 returned, 2 changed, 1 missing since the previous scan");
  });

  it("counts differences only when there is a baseline", () => {
    expect(comparisonCount(null)).toBe(0);
    expect(comparisonCount(comparison({ baseline_scan_id: null, added: [diff("new")] }))).toBe(0);
    expect(comparisonCount(comparison({ added: [diff("new")], removed: [diff("missing")] }))).toBe(2);
  });
});
