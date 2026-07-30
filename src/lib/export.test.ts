import { describe, expect, it } from "vitest";
import { buildExport, buildHostExport, exportFilename } from "./export";
import { upsertHost, type DeviceRow } from "./live";
import type { HostResult } from "../types";

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
