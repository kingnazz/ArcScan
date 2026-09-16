import { describe, expect, it } from "vitest";
import {
  DEFAULT_PORTS,
  DEPTHS,
  DEPTH_ORDER,
  PROFILES,
  PROFILE_ORDER,
  buildScanOptions,
  deepOptionsFor,
  isPrivateIpv4,
  isProfileId,
  isScanDepth,
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

describe("discovery depth", () => {
  it("lists every depth exactly once, in increasing order of effort", () => {
    expect(DEPTH_ORDER).toEqual(["quick", "deep", "credentialed"]);
    expect(DEPTH_ORDER).toHaveLength(Object.keys(DEPTHS).length);
  });

  it("gives every depth a name, a summary and a detail", () => {
    for (const id of DEPTH_ORDER) {
      const depth = DEPTHS[id];
      expect(depth.name.trim()).not.toBe("");
      expect(depth.summary.trim()).not.toBe("");
      expect(depth.detail.trim()).not.toBe("");
    }
  });

  it("recognises a depth and refuses anything else", () => {
    expect(isScanDepth("deep")).toBe(true);
    expect(isScanDepth("quick")).toBe(true);
    expect(isScanDepth("thorough")).toBe(false);
    expect(isScanDepth(null)).toBe(false);
    expect(isScanDepth(3)).toBe(false);
  });

  it("opens no deep socket at all on a Quick scan", () => {
    // The guarantee that keeps Quick Scan quick: the level is switched off,
    // not each probe individually.
    const deep = deepOptionsFor("quick");
    expect(deep.enabled).toBe(false);
    const options = buildScanOptions("192.168.1.0/24", "quick-lan");
    expect(options.deep?.enabled).toBe(false);
    expect(options.credentialed_windows).toBe(false);
  });

  it("defaults to Quick when no depth is asked for", () => {
    // A caller that predates v1.9 gets exactly the v1.8 behaviour.
    const options = buildScanOptions("192.168.1.0/24", "quick-lan");
    expect(options.deep?.enabled).toBe(false);
  });

  it("turns every probe on at Deep, and signs in only at Credentialed", () => {
    const deep = buildScanOptions("192.168.1.0/24", "quick-lan", {}, undefined, "deep");
    expect(deep.deep).toEqual({
      enabled: true,
      http: true,
      tls: true,
      smb: true,
      banners: true,
    });
    expect(deep.credentialed_windows).toBe(false);

    const credentialed = buildScanOptions(
      "192.168.1.0/24",
      "quick-lan",
      {},
      undefined,
      "credentialed",
    );
    expect(credentialed.deep?.enabled).toBe(true);
    expect(credentialed.credentialed_windows).toBe(true);
  });

  it("marks the credentialed level as needing a credential", () => {
    expect(DEPTHS.credentialed.needsCredential).toBe(true);
    expect(DEPTHS.quick.needsCredential).toBeUndefined();
    expect(DEPTHS.deep.needsCredential).toBeUndefined();
  });

  it("keeps depth independent of the profile", () => {
    // A fast sweep that then signs in to Windows is a sensible thing to want,
    // and is why depth is not folded into the profile list.
    for (const profile of PROFILE_ORDER) {
      const options = buildScanOptions("192.168.1.0/24", profile, {}, undefined, "deep");
      expect(options.deep?.enabled).toBe(true);
      expect(options.profile).toBe(profile);
    }
  });

  it("still runs deep probes on a routed scan where multicast is switched off", () => {
    // Deep probes are unicast to addresses already known to be live, so a
    // router between ArcScan and the device is not an obstacle the way it is
    // for mDNS and SSDP.
    const options = buildScanOptions("10.9.0.0/24", "remote-subnet", {}, undefined, "deep");
    expect(options.discovery?.enabled).toBe(false);
    expect(options.deep?.enabled).toBe(true);
  });
});
