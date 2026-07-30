import { describe, expect, it } from "vitest";
import {
  applyComparison,
  isStaleEvent,
  mergeHost,
  removeHostByIp,
  rowName,
  rowsFromScanDetail,
  settleRows,
  upsertHost,
  type DeviceRow,
} from "./live";
import type { HostResult, ScanComparison, ScanDetail } from "../types";

function host(ip: string, overrides: Partial<HostResult> = {}): HostResult {
  return {
    ip,
    hostname: null,
    mac: null,
    vendor: null,
    open_ports: [],
    response_ms: 2,
    icmp_ms: 1.4,
    tcp_ms: null,
    ttl: 64,
    os_guess: "Linux/Unix/macOS",
    last_seen: "2026-07-01T10:00:00Z",
    ...overrides,
  };
}

function rows(...ips: string[]): DeviceRow[] {
  return ips.reduce<DeviceRow[]>((acc, ip) => upsertHost(acc, host(ip), false), []);
}

describe("live result merging", () => {
  it("inserts hosts in ascending address order regardless of arrival order", () => {
    // Probes complete out of order, so events do not arrive sorted.
    const merged = rows("192.168.1.20", "192.168.1.3", "192.168.1.100", "192.168.1.9");
    expect(merged.map((r) => r.host.ip)).toEqual([
      "192.168.1.3",
      "192.168.1.9",
      "192.168.1.20",
      "192.168.1.100",
    ]);
  });

  it("updates an existing row instead of adding a duplicate", () => {
    let list = upsertHost([], host("10.0.0.5", { open_ports: [443] }), true);
    list = upsertHost(
      list,
      host("10.0.0.5", { mac: "AA:BB:CC:00:00:05", vendor: "Acme", hostname: "nas" }),
      false,
    );

    expect(list).toHaveLength(1);
    expect(list[0].host.mac).toBe("AA:BB:CC:00:00:05");
    expect(list[0].host.hostname).toBe("nas");
    expect(list[0].pending).toBe(false);
    // The ports from the discovery event survive the enrichment event.
    expect(list[0].host.open_ports).toEqual([443]);
  });

  it("never lets a later null erase information already reported", () => {
    const before = host("10.0.0.5", {
      hostname: "printer",
      mac: "AA:BB:CC:00:00:05",
      vendor: "HP",
      ttl: 255,
      icmp_ms: 3.2,
    });
    const after = host("10.0.0.5", {
      hostname: null,
      mac: null,
      vendor: null,
      ttl: null,
      icmp_ms: null,
      open_ports: [],
    });

    const merged = mergeHost(before, after);
    expect(merged.hostname).toBe("printer");
    expect(merged.mac).toBe("AA:BB:CC:00:00:05");
    expect(merged.vendor).toBe("HP");
    expect(merged.ttl).toBe(255);
    expect(merged.icmp_ms).toBe(3.2);
  });

  it("withdraws a host the scanner ruled out after probing", () => {
    const list = rows("10.0.0.1", "10.0.0.2", "10.0.0.3");
    const next = removeHostByIp(list, "10.0.0.2");
    expect(next.map((r) => r.host.ip)).toEqual(["10.0.0.1", "10.0.0.3"]);
    // Removing something absent is a no-op rather than an error.
    expect(removeHostByIp(next, "10.0.0.99")).toHaveLength(2);
  });

  it("settles every pending row when a scan finishes", () => {
    const list = upsertHost([], host("10.0.0.1"), true);
    expect(list[0].pending).toBe(true);
    expect(settleRows(list)[0].pending).toBe(false);
  });
});

describe("stale event rejection", () => {
  it("drops events from any scan other than the active one", () => {
    // A cancelled scan keeps emitting for a moment while its tasks wind down.
    expect(isStaleEvent(41, 42)).toBe(true);
    expect(isStaleEvent(42, 42)).toBe(false);
    // Nothing is accepted before a scan has announced itself.
    expect(isStaleEvent(42, null)).toBe(true);
  });
});

describe("comparison folding", () => {
  const comparison: ScanComparison = {
    scan_id: 2,
    baseline_scan_id: 1,
    baseline_created_at: "2026-06-30T10:00:00Z",
    baseline_target: "10.0.0.0/24",
    reason: null,
    added: [
      {
        kind: "new",
        device_id: 7,
        name: "Tablet",
        ip: "10.0.0.7",
        mac: null,
        vendor: null,
        hostname: null,
        last_seen: null,
        fields: [],
      },
    ],
    removed: [],
    changed: [
      {
        kind: "changed",
        device_id: 5,
        name: "Printer",
        ip: "10.0.0.5",
        mac: null,
        vendor: null,
        hostname: null,
        last_seen: null,
        fields: [
          {
            field: "ports",
            label: "Open services",
            from: "HTTP · 80",
            to: "HTTP · 80, HTTPS · 443",
            added_ports: [443],
            removed_ports: [],
          },
        ],
      },
    ],
  };

  it("marks new and changed rows and leaves the rest alone", () => {
    const marked = applyComparison(rows("10.0.0.5", "10.0.0.7", "10.0.0.9"), comparison);
    expect(marked.find((r) => r.host.ip === "10.0.0.7")?.change).toBe("new");
    expect(marked.find((r) => r.host.ip === "10.0.0.5")?.change).toBe("changed");
    expect(marked.find((r) => r.host.ip === "10.0.0.9")?.change).toBeNull();
  });

  it("carries the field changes through for the tooltip", () => {
    const marked = applyComparison(rows("10.0.0.5"), comparison);
    expect(marked[0].changed_fields[0].added_ports).toEqual([443]);
  });

  it("clears stale marks when a new comparison arrives", () => {
    const marked = applyComparison(rows("10.0.0.5"), comparison);
    const cleared = applyComparison(marked, { ...comparison, changed: [], added: [] });
    expect(cleared[0].change).toBeNull();
    expect(cleared[0].changed_fields).toEqual([]);
  });

  it("leaves rows untouched when there is no comparison", () => {
    const list = rows("10.0.0.5");
    expect(applyComparison(list, null)).toBe(list);
  });
});

describe("rows from a saved scan", () => {
  it("attaches device identities and sorts by address", () => {
    const detail: ScanDetail = {
      id: 3,
      target: "10.0.0.0/24",
      target_key: "cidr:10.0.0.0/24",
      profile: "quick-lan",
      created_at: "2026-07-01T10:00:00Z",
      duration_ms: 1200,
      scanned: 254,
      probed: 254,
      host_count: 2,
      new_count: 0,
      missing_count: 0,
      changed_count: 0,
      status: "completed",
      baseline_scan_id: null,
      hosts: [host("10.0.0.30"), host("10.0.0.4")],
      devices: [
        { ip: "10.0.0.30", device_id: 2, custom_name: null, status: "unclassified", first_seen: null },
        {
          ip: "10.0.0.4",
          device_id: 1,
          custom_name: "Backup NAS",
          status: "trusted",
          first_seen: "2026-01-01T00:00:00Z",
        },
      ],
    };

    const built = rowsFromScanDetail(detail);
    expect(built.map((r) => r.host.ip)).toEqual(["10.0.0.4", "10.0.0.30"]);
    expect(built[0].custom_name).toBe("Backup NAS");
    expect(built[0].status).toBe("trusted");
    expect(built[0].pending).toBe(false);
  });
});

describe("row naming", () => {
  const named = (overrides: Partial<DeviceRow>): DeviceRow => ({
    host: host("10.0.0.5"),
    device_id: null,
    custom_name: null,
    status: "unclassified",
    first_seen: null,
    change: null,
    changed_fields: [],
    pending: false,
    ...overrides,
  });

  it("prefers the operator's name, then the hostname, then the vendor", () => {
    expect(rowName(named({ custom_name: "Reception NAS" }))).toBe("Reception NAS");
    expect(rowName(named({ host: host("10.0.0.5", { hostname: "nas-01" }) }))).toBe("nas-01");
    expect(rowName(named({ host: host("10.0.0.5", { vendor: "Synology" }) }))).toBe(
      "Synology (10.0.0.5)",
    );
    expect(rowName(named({}))).toBe("10.0.0.5");
  });

  it("treats a whitespace-only name as absent", () => {
    expect(rowName(named({ custom_name: "   " }))).toBe("10.0.0.5");
  });
});
