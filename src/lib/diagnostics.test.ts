import { describe, expect, it } from "vitest";
import { MAX_REPORT_CHARS, buildDiscoveryReport, redactIp } from "./diagnostics";
import { resolveType } from "./effectiveType";
import type { DiagnosticInput } from "./diagnostics";

function input(patch: Partial<DiagnosticInput> = {}): DiagnosticInput {
  return {
    appVersion: "1.8.3",
    resolved: resolveType({ detectedType: "media_device", detectedConfidence: "medium" }),
    detectedName: "Living Room TV",
    manufacturer: "Example Corp",
    model: "TV-123",
    ouiVendor: "Example Corp",
    sources: ["mdns", "ssdp"],
    services: ["_airplay._tcp"],
    evidence: [],
    discoveryQuality: "complete",
    ip: "192.168.1.42",
    ...patch,
  };
}

describe("address redaction", () => {
  it("masks an address to its first two octets", () => {
    expect(redactIp("192.168.1.42")).toBe("192.168.x.x");
    expect(redactIp("10.0.14.9")).toBe("10.0.x.x");
  });

  it("drops anything that is not an address rather than passing it through", () => {
    for (const bogus of ["", null, undefined, "192.168.1", "fe80::1", "192.168.1.999", "hello"]) {
      expect(redactIp(bogus)).toBeNull();
    }
  });
});

describe("the discovery report", () => {
  it("says enough to fix a classification rule", () => {
    const report = buildDiscoveryReport(input());
    for (const expected of [
      "ArcScan discovery report",
      "Version: 1.8.3",
      "Device type: Media device",
      "Type source: Automatic",
      "Detected confidence: Medium",
      "Detected name: Living Room TV",
      "Model: TV-123",
      "Discovery scan state: Complete",
      "_airplay._tcp",
    ]) {
      expect(report).toContain(expected);
    }
  });

  it("carries nothing that identifies the unit, even when handed it", () => {
    const report = buildDiscoveryReport(
      input({
        evidence: [
          { source: "ssdp", kind: "serial_number", value: "SN-DEADBEEF", freshness: "current", misses: 0 },
          {
            source: "ssdp",
            kind: "url",
            value: "http://192.168.1.42:8080/desc.xml",
            freshness: "current",
            misses: 0,
          },
          {
            source: "ssdp",
            kind: "protocol_identifier",
            value: "uuid:550e8400-e29b-41d4-a716-446655440000",
            freshness: "current",
            misses: 0,
          },
          { source: "mdns", kind: "ipv6_address", value: "fe80::1", freshness: "current", misses: 0 },
          { source: "ssdp", kind: "model", value: "TV-123", freshness: "current", misses: 0 },
        ],
      }),
    );
    for (const forbidden of [
      "SN-DEADBEEF",
      "desc.xml",
      "uuid:",
      "550e8400",
      "192.168.1.42",
      "fe80::1",
    ]) {
      expect(report).not.toContain(forbidden);
    }
    // The masked address is there instead, and the usable evidence survived.
    expect(report).toContain("Address: 192.168.x.x");
    expect(report).toContain("TV-123");
  });

  it("has no field for a note, a MAC address or a database id", () => {
    const report = buildDiscoveryReport(input());
    for (const forbidden of ["Notes", "MAC address:", "Device id", "Serial"]) {
      expect(report).not.toContain(forbidden);
    }
    expect(report).toContain("omits your notes");
    expect(report).toContain("sent nowhere");
  });

  it("names a correction and keeps ArcScan's own answer beside it", () => {
    const report = buildDiscoveryReport(
      input({
        resolved: resolveType({
          userOverride: "television",
          detectedType: "media_device",
          detectedConfidence: "medium",
        }),
      }),
    );
    expect(report).toContain("Device type: Television");
    expect(report).toContain("Type source: Set by you");
    expect(report).toContain("ArcScan detected: Media device");
  });

  it("separates fresh from stale evidence and dates it in scans", () => {
    const report = buildDiscoveryReport(
      input({
        evidence: [
          { source: "mdns", kind: "service", value: "_airplay._tcp", freshness: "current", misses: 0 },
          { source: "ssdp", kind: "service", value: "MediaServer", freshness: "stale", misses: 4 },
          { source: "mdns", kind: "service", value: "_raop._tcp", freshness: "aging", misses: 1 },
        ],
      }),
    );
    const fresh = report.indexOf("Fresh evidence:");
    const stale = report.indexOf("Stale evidence:");
    expect(fresh).toBeGreaterThan(-1);
    expect(stale).toBeGreaterThan(fresh);
    expect(report).toContain("MediaServer (last seen 4 discovery scans ago)");
    expect(report).toContain("_raop._tcp (last seen 1 discovery scan ago)");
    // Aging is not stale: it belongs above the stale heading.
    expect(report.indexOf("_raop._tcp")).toBeLessThan(stale);
  });

  it("is deterministic whatever order the evidence arrives in", () => {
    const rows = [
      { source: "mdns", kind: "service", value: "_a._tcp", freshness: "current", misses: 0 },
      { source: "ssdp", kind: "model", value: "TV-123", freshness: "current", misses: 0 },
      { source: "ssdp", kind: "manufacturer", value: "Example Corp", freshness: "current", misses: 0 },
    ];
    expect(buildDiscoveryReport(input({ evidence: rows }))).toBe(
      buildDiscoveryReport(input({ evidence: [...rows].reverse() })),
    );
  });

  it("stays bounded when a device advertises a great deal", () => {
    const report = buildDiscoveryReport(
      input({
        services: Array.from({ length: 400 }, (_, i) => `_svc${i}._tcp`),
        evidence: Array.from({ length: 400 }, (_, i) => ({
          source: "mdns",
          kind: "service",
          value: `_service${i}._tcp${"x".repeat(400)}`,
          freshness: "current",
          misses: 0,
        })),
      }),
    );
    expect([...report].length).toBeLessThanOrEqual(MAX_REPORT_CHARS + 32);
    // Each line is bounded too, so one hostile value cannot dominate.
    for (const line of report.split("\n")) {
      expect([...line].length).toBeLessThanOrEqual(240);
    }
  });

  it("flattens control characters a device put in its own strings", () => {
    const report = buildDiscoveryReport(input({ model: "TV\u0000-\n123\u0007" }));
    expect(report).toContain("TV - 123");
    expect(report).not.toContain("\u0000");
    expect(report).not.toContain("\u0007");
  });
});
