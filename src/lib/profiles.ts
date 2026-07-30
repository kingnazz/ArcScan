// Scan profiles.
//
// A profile is a named bundle of the settings an operator would otherwise have
// to reason about individually. It exists so the common cases are one click and
// the advanced controls stay available rather than becoming the default
// experience.
//
// Profile ids are persisted with every scan and are what decides whether two
// scans may be compared, so they are stable strings and must not be renamed.

import type { ScanOptions } from "../types";

export type ProfileId = "quick-lan" | "reliable-lan" | "full-tcp" | "remote-subnet" | "custom";

export interface ScanProfile {
  id: ProfileId;
  name: string;
  /** One line, shown next to the name in the profile picker. */
  summary: string;
  /** What the profile is for, shown in the picker's expanded description. */
  detail: string;
  ports: number[];
  timeout_ms: number;
  concurrency: number;
  tcp_concurrency: number;
  ping_concurrency: number;
  /** null lets the backend decide from the detected subnets and the ARP cache. */
  arp_assist: boolean | null;
  /** Full TCP asks the operator for a range instead of using a fixed set. */
  wants_port_range?: boolean;
}

/** The curated default port set, matching `ports::DEFAULT_PORTS` in Rust. */
export const DEFAULT_PORTS = [21, 22, 23, 53, 80, 110, 139, 143, 443, 445, 3389, 5900, 8080, 8443];

/** A wider set for the thorough profile: printers, casting, IoT and management. */
const RELIABLE_PORTS = [
  21, 22, 23, 53, 80, 135, 139, 143, 443, 445, 515, 548, 554, 631, 1900, 3389, 5000, 5353, 5900,
  8080, 8443, 9100,
];

export const PROFILES: Record<ProfileId, ScanProfile> = {
  "quick-lan": {
    id: "quick-lan",
    name: "Quick LAN",
    summary: "Fast local discovery",
    detail:
      "Sweeps your local subnet with the common service ports. The right choice for a routine look at what is connected.",
    ports: DEFAULT_PORTS,
    timeout_ms: 700,
    concurrency: 64,
    tcp_concurrency: 256,
    ping_concurrency: 32,
    arp_assist: null,
  },
  "reliable-lan": {
    id: "reliable-lan",
    name: "Reliable LAN",
    summary: "Slower, finds quiet devices",
    detail:
      "Longer timeouts, a gentler sweep and a wider port set. Use it for phones, printers, Wi-Fi devices, IoT equipment and anything that ignores the first probe.",
    ports: RELIABLE_PORTS,
    timeout_ms: 1600,
    concurrency: 32,
    tcp_concurrency: 128,
    ping_concurrency: 24,
    arp_assist: null,
  },
  "full-tcp": {
    id: "full-tcp",
    name: "Full TCP",
    summary: "Your own port range",
    detail:
      "Probes a port range you choose. Large ranges across many addresses take a long time and put sustained load on your network, so ArcScan shows the workload before it starts.",
    ports: [],
    timeout_ms: 900,
    concurrency: 32,
    tcp_concurrency: 256,
    ping_concurrency: 24,
    arp_assist: null,
    wants_port_range: true,
  },
  "remote-subnet": {
    id: "remote-subnet",
    name: "Remote subnet",
    summary: "Routed networks, no ARP",
    detail:
      "ICMP and TCP discovery with no local ARP assumptions, for networks on the other side of a router or VPN. Devices that drop every probe cannot be found this way.",
    ports: DEFAULT_PORTS,
    timeout_ms: 1200,
    concurrency: 48,
    tcp_concurrency: 192,
    ping_concurrency: 24,
    arp_assist: false,
  },
  custom: {
    id: "custom",
    name: "Custom",
    summary: "Every control, yours to set",
    detail:
      "Choose the ports, timeout and all three concurrency limits yourself. Nothing is decided for you.",
    ports: DEFAULT_PORTS,
    timeout_ms: 900,
    concurrency: 64,
    tcp_concurrency: 256,
    ping_concurrency: 32,
    arp_assist: null,
  },
};

export const PROFILE_ORDER: ProfileId[] = [
  "quick-lan",
  "reliable-lan",
  "full-tcp",
  "remote-subnet",
  "custom",
];

export function isProfileId(value: unknown): value is ProfileId {
  return typeof value === "string" && value in PROFILES;
}

/** Human name for a profile id read back from a saved scan. */
export function profileName(id: string | null | undefined): string {
  if (!id) return "Custom";
  return isProfileId(id) ? PROFILES[id].name : id;
}

/** The settings a profile overrides, for building scan options. */
export interface ProfileOverrides {
  ports?: number[];
  timeout_ms?: number;
  concurrency?: number;
  tcp_concurrency?: number;
  ping_concurrency?: number;
}

/**
 * Build the scan options for a target and profile.
 *
 * Overrides come from the advanced panel. Only the Custom and Full TCP profiles
 * let them through: a named profile that quietly ran with different limits would
 * make its scans incomparable with earlier ones bearing the same name.
 */
export function buildScanOptions(
  target: string,
  profileId: ProfileId,
  overrides: ProfileOverrides = {},
): ScanOptions {
  const profile = PROFILES[profileId];
  const tunable = profileId === "custom" || profileId === "full-tcp";
  const pick = <K extends keyof ProfileOverrides>(key: K, fallback: number): number => {
    const value = tunable ? overrides[key] : undefined;
    return typeof value === "number" ? value : fallback;
  };

  const ports = tunable && overrides.ports?.length ? overrides.ports : profile.ports;

  return {
    target: target.trim(),
    ports,
    timeout_ms: pick("timeout_ms", profile.timeout_ms),
    concurrency: pick("concurrency", profile.concurrency),
    tcp_concurrency: pick("tcp_concurrency", profile.tcp_concurrency),
    ping_concurrency: pick("ping_concurrency", profile.ping_concurrency),
    profile: profileId,
    arp_assist: profile.arp_assist,
  };
}

/**
 * Recommend a profile for a target.
 *
 * A private address is very likely the operator's own segment, where ARP-assisted
 * discovery is what finds everything. Anything else is treated as routed, where
 * local ARP assumptions do not hold.
 */
export function recommendedProfile(target: string, localCidrs: string[] = []): ProfileId {
  const trimmed = target.trim();
  if (!trimmed) return "quick-lan";
  if (localCidrs.some((cidr) => cidr === trimmed)) return "quick-lan";
  const first = trimmed.split(/[/\-\s]/)[0];
  return isPrivateIpv4(first) ? "quick-lan" : "remote-subnet";
}

/** RFC 1918 plus the shared-address and link-local ranges a LAN might use. */
export function isPrivateIpv4(ip: string): boolean {
  const parts = ip.split(".");
  if (parts.length !== 4) return false;
  const [a, b] = parts.map((p) => Number.parseInt(p, 10));
  if (!Number.isFinite(a) || !Number.isFinite(b)) return false;
  if (a === 10) return true;
  if (a === 172 && b >= 16 && b <= 31) return true;
  if (a === 192 && b === 168) return true;
  if (a === 169 && b === 254) return true;
  if (a === 100 && b >= 64 && b <= 127) return true;
  return false;
}
