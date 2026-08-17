// Partial-scan rules as the frontend sees them: the mock backend mirrors the
// Rust rules (cancelled scans are saved but never compared and never become
// baselines), and the history panel labels partial scans and disables their
// comparison with an explanation.

import { describe, expect, it } from "vitest";
import { PARTIAL_SCAN_REASON, mock } from "./mock";
import { compareUnavailableReason } from "../components/HistoryPanel";
import type { HostResult, ScanResult, ScanSummary } from "../types";

function host(ip: string, ports: number[]): HostResult {
  return {
    ip,
    hostname: null,
    mac: `AA:BB:CC:00:00:${ip.split(".")[3].padStart(2, "0")}`,
    vendor: "Acme",
    open_ports: ports,
    response_ms: 2,
    icmp_ms: 1.5,
    tcp_ms: 2.5,
    ttl: 64,
    os_guess: "Linux/Unix/macOS",
    last_seen: new Date().toISOString(),
  };
}

function result(hosts: HostResult[], cancelled: boolean): ScanResult {
  return {
    scan_id: 1,
    target: "172.16.9.0/24",
    profile: "quick-lan",
    duration_ms: 900,
    scanned: 254,
    probed: cancelled ? 40 : 254,
    hosts,
    cancelled,
    ports: [22, 80, 443],
    arp_assist: null,
  };
}

describe("partial scans in the mock backend", () => {
  it("saves a cancelled scan but reports no changes for it", () => {
    mock.save(result([host("172.16.9.5", [445]), host("172.16.9.6", [22])], false));

    // The cancelled scan reached only one device with fewer ports; that must
    // not read as one missing device and one closed port.
    const partial = mock.save(result([host("172.16.9.5", [])], true));
    expect(partial.comparison.baseline_scan_id).toBeNull();
    expect(partial.comparison.reason).toBe(PARTIAL_SCAN_REASON);
    expect(partial.comparison.removed).toEqual([]);
    expect(partial.comparison.changed).toEqual([]);

    const summary = mock.listScans().find((s) => s.id === partial.scan_id);
    expect(summary?.status).toBe("cancelled");
    expect(summary?.missing_count).toBe(0);
    expect(summary?.changed_count).toBe(0);
    // The partial results themselves are saved and reopenable.
    expect(mock.getScan(partial.scan_id).hosts).toHaveLength(1);
  });

  it("skips cancelled scans when picking a baseline", () => {
    const first = mock.save(result([host("172.16.9.5", [445])], false));
    mock.save(result([host("172.16.9.5", [445])], true));
    const second = mock.save(result([host("172.16.9.5", [445])], false));
    expect(second.comparison.baseline_scan_id).toBe(first.scan_id);
  });

  it("re-asking for a cancelled scan's comparison still explains the partiality", () => {
    const partial = mock.save(result([host("172.16.9.7", [80])], true));
    const comparison = mock.compareScan(partial.scan_id);
    expect(comparison.reason).toBe(PARTIAL_SCAN_REASON);
    expect(comparison.baseline_scan_id).toBeNull();
  });
});

describe("history labels for partial scans", () => {
  const summary = (overrides: Partial<ScanSummary>): ScanSummary => ({
    id: 1,
    target: "10.0.0.0/24",
    target_key: "cidr:10.0.0.0/24",
    profile: "quick-lan",
    discovery_mode: "full",
    discovery_summary: null,
    created_at: "2026-07-01T10:00:00Z",
    duration_ms: 900,
    scanned: 254,
    probed: 254,
    host_count: 3,
    new_count: 0,
    missing_count: 0,
    changed_count: 0,
    status: "completed",
    baseline_scan_id: 5,
    network_scope_id: 1,
    scope_name: "Office network",
    coverage_key: "v1|arp:auto|ports:22,80,443",
    ...overrides,
  });

  it("disables comparison for a cancelled scan with an explanation", () => {
    const reason = compareUnavailableReason(summary({ status: "cancelled", baseline_scan_id: null }));
    expect(reason).toMatch(/stopped early/);
  });

  it("explains a missing baseline for a completed scan", () => {
    const reason = compareUnavailableReason(summary({ baseline_scan_id: null }));
    expect(reason).toMatch(/No earlier completed scan/);
  });

  it("allows comparison when a compatible baseline exists", () => {
    expect(compareUnavailableReason(summary({}))).toBeNull();
  });
});
