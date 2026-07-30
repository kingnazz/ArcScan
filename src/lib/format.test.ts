import { afterEach, describe, expect, it } from "vitest";
import {
  formatDuration,
  formatLatency,
  ipToNum,
  isSensitivePort,
  parsePorts,
  phaseLabel,
  serviceLabel,
  serviceWithPort,
  setServiceCatalog,
  webPort,
} from "./format";

describe("port specification parsing", () => {
  it("accepts single ports, lists and ranges in any mix", () => {
    expect(parsePorts("443").ports).toEqual([443]);
    expect(parsePorts("22,80,443").ports).toEqual([22, 80, 443]);
    expect(parsePorts("22 80 443").ports).toEqual([22, 80, 443]);
    expect(parsePorts("80-82").ports).toEqual([80, 81, 82]);
    expect(parsePorts("443, 80-82, 22").ports).toEqual([22, 80, 81, 82, 443]);
  });

  it("de-duplicates and sorts", () => {
    expect(parsePorts("443,80,443,80-81").ports).toEqual([80, 81, 443]);
  });

  it("accepts a reversed range and orders it", () => {
    expect(parsePorts("82-80").ports).toEqual([80, 81, 82]);
  });

  it("reports the offending token rather than failing silently", () => {
    expect(parsePorts("0").error).toMatch(/Ports are 1 to 65535/);
    expect(parsePorts("65536").error).toMatch(/Ports are 1 to 65535/);
    expect(parsePorts("http").error).toMatch(/"http" is not a port number/);
    expect(parsePorts("80,http").error).toMatch(/"http"/);
    // A rejected spec yields no ports, so a bad value cannot reach a scan.
    expect(parsePorts("80,http").ports).toEqual([]);
  });

  it("refuses to exceed the cap instead of truncating", () => {
    const result = parsePorts("1-65535");
    expect(result.error).toMatch(/2,048 port limit/);
    expect(result.ports).toEqual([]);
    expect(parsePorts("1-2048").ports).toHaveLength(2048);
    expect(parsePorts("1-2048,3000").error).toMatch(/2,048/);
  });

  it("treats an empty spec as no opinion rather than an error", () => {
    expect(parsePorts("   ")).toEqual({ ports: [], error: null });
  });
});

describe("service labels", () => {
  afterEach(() => {
    // Restore the built-in table so one test cannot leak into the next.
    setServiceCatalog([
      { port: 22, name: "SSH", sensitive: false },
      { port: 443, name: "HTTPS", sensitive: false },
      { port: 3389, name: "RDP", sensitive: true },
      { port: 445, name: "SMB", sensitive: true },
      { port: 80, name: "HTTP", sensitive: false },
      { port: 8443, name: "HTTPS-alt", sensitive: false },
      { port: 8080, name: "HTTP-alt", sensitive: false },
    ]);
  });

  it("falls back to the port number when no name is known", () => {
    expect(serviceLabel(443)).toBe("HTTPS");
    expect(serviceLabel(64999)).toBe("64999");
    expect(serviceWithPort(443)).toBe("HTTPS · 443");
    expect(serviceWithPort(64999)).toBe("64999");
  });

  it("takes the backend's catalog when it arrives", () => {
    setServiceCatalog([{ port: 9999, name: "Custom Thing", sensitive: true }]);
    expect(serviceLabel(9999)).toBe("Custom Thing");
    expect(isSensitivePort(9999)).toBe(true);
  });

  it("ignores an empty catalog so the fallback survives a failed fetch", () => {
    setServiceCatalog([]);
    expect(serviceLabel(443)).toBe("HTTPS");
  });

  it("flags remote-access services and not ordinary ones", () => {
    expect(isSensitivePort(3389)).toBe(true);
    expect(isSensitivePort(445)).toBe(true);
    expect(isSensitivePort(443)).toBe(false);
  });
});

describe("web interface selection", () => {
  it("prefers HTTPS over plain HTTP", () => {
    expect(webPort([80, 443])).toBe(443);
    expect(webPort([80, 8443])).toBe(8443);
    expect(webPort([80, 8080])).toBe(80);
    expect(webPort([8080])).toBe(8080);
  });

  it("returns null when nothing serves a web interface", () => {
    expect(webPort([22, 445])).toBeNull();
    expect(webPort([])).toBeNull();
  });
});

describe("formatting", () => {
  it("orders addresses numerically", () => {
    expect(ipToNum("10.0.0.57")).toBeLessThan(ipToNum("10.0.0.200"));
    expect(ipToNum("nonsense")).toBe(0);
  });

  it("keeps precision where it means something and drops it where it does not", () => {
    expect(formatLatency(0.84)).toBe("0.84 ms");
    expect(formatLatency(2.45)).toBe("2.5 ms");
    expect(formatLatency(180.4)).toBe("180 ms");
    expect(formatLatency(null)).toBeNull();
    expect(formatLatency(Number.NaN)).toBeNull();
  });

  it("scales durations from milliseconds to minutes", () => {
    expect(formatDuration(420)).toBe("420 ms");
    expect(formatDuration(4_200)).toBe("4.2 s");
    expect(formatDuration(95_000)).toBe("1m 35s");
  });

  it("names every scan phase the backend can report", () => {
    expect(phaseLabel("probing")).toBe("Probing addresses");
    expect(phaseLabel("confirming")).toBe("Confirming quiet devices");
    expect(phaseLabel("resolving")).toBe("Resolving names and vendors");
    expect(phaseLabel("done")).toBe("Finished");
    expect(phaseLabel("cancelled")).toBe("Stopped");
    // An unknown phase from a newer backend still reads as something sensible.
    expect(phaseLabel("teleporting")).toBe("Scanning");
  });
});
