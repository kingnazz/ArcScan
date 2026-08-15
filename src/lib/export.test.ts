import { describe, expect, it } from "vitest";
import {
  buildChangesExport,
  buildExport,
  buildHostExport,
  buildInventoryExport,
  datedFilename,
  exportFilename,
} from "./export";
import { upsertHost, type DeviceRow } from "./live";
import type { ChangeEvent, HostResult, InventoryRow } from "../types";

function host(overrides: Partial<HostResult> = {}): HostResult {
  return {
    ip: "10.0.0.5",
    hostname: "nas-backup",
    mac: "AA:BB:CC:00:00:05",
    vendor: "Synology Incorporated",
    open_ports: [22, 443, 445],
    response_ms: 2,
    icmp_ms: 1.2,
    tcp_ms: 2.6,
    ttl: 64,
    os_guess: "Linux/Unix/macOS",
    last_seen: "2026-07-01T10:00:00Z",
    ...overrides,
  };
}

function row(h: HostResult, overrides: Partial<DeviceRow> = {}): DeviceRow {
  const [built] = upsertHost([], h, false);
  return { ...built, ...overrides };
}

describe("CSV export", () => {
  it("writes a header and one line per device", () => {
    const csv = buildExport([row(host()), row(host({ ip: "10.0.0.6" }))], "csv");
    const lines = csv.trimEnd().split("\n");
    expect(lines).toHaveLength(3);
    expect(lines[0]).toBe(
      "Name,IP,Hostname,MAC,Vendor,OS,TTL,Open Ports,Response (ms),ICMP (ms),TCP (ms),Status,Last Seen",
    );
    expect(lines[1]).toContain("10.0.0.5");
    expect(lines[1]).toContain("22 443 445");
  });

  it("quotes and escapes fields that would break the format", () => {
    // Vendor names really do contain commas, and a device name is operator input.
    const csv = buildExport(
      [
        row(host({ vendor: "TP-LINK TECHNOLOGIES CO.,LTD." }), {
          custom_name: 'Reception "main" desk',
        }),
      ],
      "csv",
    );
    expect(csv).toContain('"TP-LINK TECHNOLOGIES CO.,LTD."');
    expect(csv).toContain('"Reception ""main"" desk"');
  });

  it("leaves missing values empty rather than writing null", () => {
    const csv = buildExport(
      [row(host({ hostname: null, mac: null, vendor: null, os_guess: null, ttl: null }))],
      "csv",
    );
    expect(csv).not.toMatch(/null|undefined/);
  });

  it("writes only a header for an empty result set", () => {
    expect(buildExport([], "csv").trimEnd().split("\n")).toHaveLength(1);
  });

  it("uses the operator's name in the Name column", () => {
    const csv = buildExport([row(host(), { custom_name: "Backup NAS" })], "csv");
    expect(csv).toContain("Backup NAS");
  });
});

describe("JSON export", () => {
  it("produces parseable JSON with both latency measurements", () => {
    const parsed = JSON.parse(buildExport([row(host())], "json")) as Array<Record<string, string>>;
    expect(parsed).toHaveLength(1);
    expect(parsed[0].ip).toBe("10.0.0.5");
    expect(parsed[0].icmp_ms).toBe("1.2");
    expect(parsed[0].tcp_ms).toBe("2.6");
    expect(parsed[0].response_ms).toBe("2");
  });

  it("produces an empty array for no devices", () => {
    expect(JSON.parse(buildExport([], "json"))).toEqual([]);
  });
});

describe("XML export", () => {
  it("escapes markup so a hostname cannot break the document", () => {
    const xml = buildExport([row(host({ hostname: 'a<b>&"c' }))], "xml");
    expect(xml).toContain("&lt;b&gt;");
    expect(xml).toContain("&amp;");
    expect(xml).toContain("&quot;");
    expect(xml).not.toMatch(/<b>/);
  });

  it("declares the encoding and wraps the devices", () => {
    const xml = buildExport([row(host())], "xml");
    expect(xml.startsWith('<?xml version="1.0" encoding="UTF-8"?>')).toBe(true);
    expect(xml).toContain("<devices>");
    expect(xml).toContain("<device>");
  });
});

describe("host export", () => {
  it("works from raw host results with no row metadata", () => {
    const csv = buildHostExport([host()], "csv");
    // With no operator name, the hostname becomes the display name.
    expect(csv).toContain("nas-backup");
  });
});

describe("export filenames", () => {
  it("makes a target safe for a filesystem and keeps the extension", () => {
    const name = exportFilename("192.168.1.0/24", "csv");
    expect(name).toMatch(/^arcscan-192\.168\.1\.0_24-[\d-]+\.csv$/);
    expect(name).not.toContain("/");
  });

  it("falls back to a generic name for an unusable target", () => {
    expect(exportFilename("///", "json")).toMatch(/^arcscan-scan-[\d-]+\.json$/);
  });
});

// ---------------------------------------------------------------------------
// Inventory and Changes exports (v1.8)
// ---------------------------------------------------------------------------

function inventoryRow(patch: Partial<InventoryRow> = {}): InventoryRow {
  return {
    device_id: 3,
    network_scope_id: 1,
    network_name: "Home Wi-Fi",
    identity_source: "mac",
    display_name: "Office Printer",
    custom_name: "Office Printer",
    hostname: "office-printer",
    current_ip: "192.168.1.31",
    previous_ips: ["192.168.1.28", "192.168.1.20"],
    mac: "3C:D9:2B:6F:08:AA",
    vendor: "Hewlett Packard",
    os_guess: "Network device",
    status: "trusted",
    presence: "missing",
    first_seen: "2026-07-01T09:00:00Z",
    last_seen: "2026-08-02T09:00:00Z",
    last_completed_scan_id: 4,
    last_completed_scan_at: "2026-08-02T09:00:00Z",
    observation_count: 6,
    open_ports: [80, 443],
    notes_present: true,
    notes_excerpt: "Toner reordered",
    latest_response_ms: 9,
    latest_icmp_ms: 8.9,
    latest_tcp_ms: 10.3,
    ...patch,
  };
}

function changeEvent(patch: Partial<ChangeEvent> = {}): ChangeEvent {
  return {
    id: 11,
    event_key: "s2|d3|ports_changed",
    scan_id: 2,
    baseline_scan_id: 1,
    network_scope_id: 1,
    network_name: "Home Wi-Fi",
    device_id: 3,
    device_label: "Home NAS",
    ip: "192.168.1.50",
    mac: "00:11:32:5D:A2:77",
    vendor: "Synology Incorporated",
    change_type: "ports_changed",
    old_value: "SSH · 22",
    new_value: "SSH · 22, HTTPS · 443",
    opened_ports: [443],
    closed_ports: [22],
    state: "acknowledged",
    created_at: "2026-08-02T09:00:00Z",
    scan_at: "2026-08-02T09:00:00Z",
    baseline_at: "2026-07-27T09:00:00Z",
    acknowledged_at: "2026-08-03T08:00:00Z",
    device_status: "trusted",
    ...patch,
  };
}

describe("inventory export", () => {
  it("writes the documented columns, in order", () => {
    const csv = buildInventoryExport([inventoryRow()], "csv");
    const lines = csv.trimEnd().split("\n");
    expect(lines[0]).toBe(
      "Network,Device,Status,Presence,Current IP,Previous IPs,MAC,Manufacturer,Hostname,OS guess,Open ports,Open services,First seen,Last seen,Observations,Detected name,Device type,Type confidence,Discovered by,Detected manufacturer,Model,Advertised services,Last discovered,Notes",
    );
    expect(lines).toHaveLength(2);
  });

  it("spells presence and status out rather than exporting internal words", () => {
    const csv = buildInventoryExport([inventoryRow()], "csv");
    expect(csv).toContain("Missing from latest scan");
    expect(csv).toContain("Trusted");
    // Never the raw enum value.
    expect(csv).not.toContain(",missing,");
    expect(csv).not.toContain(",unclassified,");
  });

  it("maps the unclassified status to the word the interface uses", () => {
    const csv = buildInventoryExport([inventoryRow({ status: "unclassified" })], "csv");
    expect(csv).toContain("Unreviewed");
  });

  it("carries every previous address and both port forms", () => {
    const csv = buildInventoryExport([inventoryRow()], "csv");
    expect(csv).toContain("192.168.1.28 192.168.1.20");
    expect(csv).toContain("80 443");
    expect(csv).toContain("HTTP · 80, HTTPS · 443");
  });

  it("includes note bodies the caller fetched, and nothing when there are none", () => {
    const withNotes = buildInventoryExport(
      [inventoryRow()],
      "csv",
      new Map([[3, "Toner reordered automatically"]]),
    );
    expect(withNotes).toContain("Toner reordered automatically");
    expect(buildInventoryExport([inventoryRow()], "csv")).not.toContain("Toner reordered");
  });

  it("keeps internal ids out of CSV and XML, and only in JSON", () => {
    expect(buildInventoryExport([inventoryRow()], "csv")).not.toContain("device_id");
    expect(buildInventoryExport([inventoryRow()], "xml")).not.toContain("device_id");
    const json = JSON.parse(buildInventoryExport([inventoryRow()], "json"));
    expect(json[0].device_id).toBe(3);
    expect(json[0].presence).toBe("Missing from latest scan");
  });

  it("produces well-formed XML with one element per device", () => {
    const xml = buildInventoryExport([inventoryRow(), inventoryRow({ device_id: 4 })], "xml");
    expect(xml.startsWith('<?xml version="1.0" encoding="UTF-8"?>')).toBe(true);
    expect(xml).toContain("<inventory>");
    expect(xml.match(/<device>/g)).toHaveLength(2);
  });

  it("writes an empty but valid document when nothing is selected", () => {
    expect(buildInventoryExport([], "csv").trimEnd().split("\n")).toHaveLength(1);
    expect(buildInventoryExport([], "xml")).toContain("</inventory>");
    expect(JSON.parse(buildInventoryExport([], "json"))).toEqual([]);
  });
});

describe("changes export", () => {
  it("writes the documented columns, in order", () => {
    const csv = buildChangesExport([changeEvent()], "csv");
    expect(csv.trimEnd().split("\n")[0]).toBe(
      "Date,Network,Device,IP,MAC,Change,Previous value,New value,Opened ports,Closed ports,Scan,Baseline,Review state,Acknowledged",
    );
  });

  it("carries the review state, the acknowledgement date and both scans", () => {
    const csv = buildChangesExport([changeEvent()], "csv");
    expect(csv).toContain("Acknowledged");
    expect(csv).toContain("2026-08-03T08:00:00Z");
    expect(csv).toContain("Service change");
    expect(csv).toContain("443");
    expect(csv).toContain("22");
  });

  it("exports exactly the events it is given, ignored ones included", () => {
    const events = [changeEvent({ id: 1 }), changeEvent({ id: 2, state: "ignored" })];
    const csv = buildChangesExport(events, "csv");
    expect(csv.trimEnd().split("\n")).toHaveLength(3);
    expect(csv).toContain("Ignored");
  });

  it("leaves a pruned scan's columns blank rather than inventing an id", () => {
    const csv = buildChangesExport([changeEvent({ scan_id: null, baseline_scan_id: null })], "csv");
    expect(csv).toContain(",,,Acknowledged");
  });
});

describe("dated filenames", () => {
  it("names the kind and the day", () => {
    const name = datedFilename("inventory", null, "csv");
    expect(name).toMatch(/^arcscan-inventory-\d{4}-\d{2}-\d{2}\.csv$/);
    expect(datedFilename("changes", null, "json")).toMatch(
      /^arcscan-changes-\d{4}-\d{2}-\d{2}\.json$/,
    );
  });

  it("carries the network so a folder of exports explains itself", () => {
    expect(datedFilename("inventory", "Home Wi-Fi", "csv")).toMatch(
      /^arcscan-inventory-home-wi-fi-\d{4}-\d{2}-\d{2}\.csv$/,
    );
  });

  it("collapses a name made entirely of punctuation instead of leaving dashes", () => {
    expect(datedFilename("inventory", "///", "csv")).toMatch(
      /^arcscan-inventory-\d{4}-\d{2}-\d{2}\.csv$/,
    );
  });
});
