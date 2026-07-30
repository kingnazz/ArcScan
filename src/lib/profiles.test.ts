import { describe, expect, it } from "vitest";
import {
  DEFAULT_PORTS,
  PROFILES,
  PROFILE_ORDER,
  buildScanOptions,
  isPrivateIpv4,
  isProfileId,
  profileName,
  recommendedProfile,
} from "./profiles";

describe("profile catalogue", () => {
  it("lists every profile exactly once", () => {
    expect(PROFILE_ORDER).toHaveLength(Object.keys(PROFILES).length);
    expect(new Set(PROFILE_ORDER).size).toBe(PROFILE_ORDER.length);
  });

  it("gives every profile a name, a summary and a detail", () => {
    for (const id of PROFILE_ORDER) {
      const profile = PROFILES[id];
      expect(profile.name, id).toBeTruthy();
      expect(profile.summary, id).toBeTruthy();
      expect(profile.detail.length, id).toBeGreaterThan(30);
    }
  });

  it("keeps every profile's limits inside what the backend accepts", () => {
    for (const id of PROFILE_ORDER) {
      const p = PROFILES[id];
      expect(p.timeout_ms, id).toBeGreaterThanOrEqual(50);
      expect(p.timeout_ms, id).toBeLessThanOrEqual(10_000);
      expect(p.concurrency, id).toBeGreaterThanOrEqual(1);
      expect(p.concurrency, id).toBeLessThanOrEqual(1_024);
      expect(p.tcp_concurrency, id).toBeGreaterThanOrEqual(8);
      expect(p.tcp_concurrency, id).toBeLessThanOrEqual(2_048);
      expect(p.ping_concurrency, id).toBeGreaterThanOrEqual(1);
      expect(p.ping_concurrency, id).toBeLessThanOrEqual(128);
      expect(p.ports.length, id).toBeLessThanOrEqual(2_048);
    }
  });

  it("makes Reliable LAN genuinely gentler than Quick LAN", () => {
    const quick = PROFILES["quick-lan"];
    const reliable = PROFILES["reliable-lan"];
    expect(reliable.timeout_ms).toBeGreaterThan(quick.timeout_ms);
    expect(reliable.concurrency).toBeLessThan(quick.concurrency);
    expect(reliable.tcp_concurrency).toBeLessThan(quick.tcp_concurrency);
    expect(reliable.ports.length).toBeGreaterThan(quick.ports.length);
  });

  it("has Remote subnet opt out of local ARP assumptions", () => {
    expect(PROFILES["remote-subnet"].arp_assist).toBe(false);
    // Every other profile lets the backend decide from the detected subnets.
    for (const id of PROFILE_ORDER.filter((p) => p !== "remote-subnet")) {
      expect(PROFILES[id].arp_assist, id).toBeNull();
    }
  });

  it("recognises and names profile ids read back from saved scans", () => {
    expect(isProfileId("quick-lan")).toBe(true);
    expect(isProfileId("nonsense")).toBe(false);
    expect(profileName("full-tcp")).toBe("Full TCP");
    // An unknown or absent profile still produces something printable.
    expect(profileName(null)).toBe("Custom");
    expect(profileName("from-a-newer-version")).toBe("from-a-newer-version");
  });
});

describe("building scan options", () => {
  it("uses the profile's own settings and ignores overrides for named profiles", () => {
    // A named profile that quietly ran with different limits would make its
    // scans incomparable with earlier ones bearing the same name.
    const opts = buildScanOptions("192.168.1.0/24", "quick-lan", {
      ports: [1, 2, 3],
      timeout_ms: 5_000,
      concurrency: 512,
      tcp_concurrency: 2_000,
      ping_concurrency: 100,
    });
    expect(opts.ports).toEqual(PROFILES["quick-lan"].ports);
    expect(opts.timeout_ms).toBe(PROFILES["quick-lan"].timeout_ms);
    expect(opts.concurrency).toBe(PROFILES["quick-lan"].concurrency);
    expect(opts.profile).toBe("quick-lan");
  });

  it("lets Custom and Full TCP take the operator's overrides", () => {
    for (const id of ["custom", "full-tcp"] as const) {
      const opts = buildScanOptions("10.0.0.0/24", id, {
        ports: [1, 2, 3],
        timeout_ms: 1_500,
        concurrency: 16,
        tcp_concurrency: 64,
        ping_concurrency: 8,
      });
      expect(opts.ports, id).toEqual([1, 2, 3]);
      expect(opts.timeout_ms, id).toBe(1_500);
      expect(opts.concurrency, id).toBe(16);
      expect(opts.tcp_concurrency, id).toBe(64);
      expect(opts.ping_concurrency, id).toBe(8);
    }
  });

  it("falls back to the defaults when an override is missing or empty", () => {
    const opts = buildScanOptions("10.0.0.0/24", "custom", { ports: [] });
    expect(opts.ports).toEqual(DEFAULT_PORTS);
    expect(opts.timeout_ms).toBe(PROFILES.custom.timeout_ms);
  });

  it("trims the target so a pasted value with whitespace still scans", () => {
    expect(buildScanOptions("  192.168.1.0/24 \n", "quick-lan").target).toBe("192.168.1.0/24");
  });
});

describe("profile recommendation", () => {
  it("recommends Quick LAN for private targets and Remote subnet for routed ones", () => {
    expect(recommendedProfile("192.168.1.0/24")).toBe("quick-lan");
    expect(recommendedProfile("10.0.0.1-50")).toBe("quick-lan");
    expect(recommendedProfile("172.16.4.9")).toBe("quick-lan");
    expect(recommendedProfile("8.8.8.8")).toBe("remote-subnet");
    expect(recommendedProfile("203.0.113.0/24")).toBe("remote-subnet");
  });

  it("recommends Quick LAN for a detected local network whatever its addresses", () => {
    expect(recommendedProfile("100.64.0.0/24", ["100.64.0.0/24"])).toBe("quick-lan");
  });

  it("defaults to Quick LAN for an empty target", () => {
    expect(recommendedProfile("   ")).toBe("quick-lan");
  });

  it("identifies the private ranges a LAN actually uses", () => {
    expect(isPrivateIpv4("10.1.2.3")).toBe(true);
    expect(isPrivateIpv4("172.16.0.1")).toBe(true);
    expect(isPrivateIpv4("172.31.255.254")).toBe(true);
    expect(isPrivateIpv4("172.32.0.1")).toBe(false);
    expect(isPrivateIpv4("192.168.0.1")).toBe(true);
    expect(isPrivateIpv4("169.254.1.1")).toBe(true);
    expect(isPrivateIpv4("100.100.0.1")).toBe(true);
    expect(isPrivateIpv4("8.8.8.8")).toBe(false);
    expect(isPrivateIpv4("not-an-ip")).toBe(false);
  });
});
