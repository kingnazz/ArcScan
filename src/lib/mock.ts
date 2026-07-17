// Pure-TypeScript mock scanner. Runs entirely in the browser with no Rust
// backend so the UI is fully developable and demoable. The API layer falls
// back to this automatically when the app is not running inside Tauri.

import type { HostResult, ScanOptions, ScanResult } from "../types";

// A small in-memory "history" so the mock supports save/list/get/delete too.
interface MockScan extends ScanResult {
  id: number;
  created_at: string;
}

const store: MockScan[] = [];
let nextId = 1;

const VENDORS: Array<[string, string]> = [
  ["Ubiquiti Inc", "80:2A:A8"],
  ["Apple, Inc.", "F0:18:98"],
  ["Dell Inc.", "18:66:DA"],
  ["Hewlett Packard", "3C:D9:2B"],
  ["Cisco Systems, Inc", "00:1B:D4"],
  ["TP-LINK TECHNOLOGIES CO.,LTD.", "50:C7:BF"],
  ["Intel Corporate", "94:C6:91"],
  ["Samsung Electronics", "8C:77:12"],
  ["Raspberry Pi Foundation", "B8:27:EB"],
  ["Synology Incorporated", "00:11:32"],
  ["Microsoft Corporation", "00:15:5D"],
  ["Amazon Technologies", "FC:65:DE"],
];

const HOSTNAMES = [
  "gateway",
  "nas01",
  "ws-reception",
  "ws-accounting",
  "printer-hp",
  "ap-lobby",
  "srv-dc01",
  "cam-frontdoor",
  "tv-conference",
  "voip-desk-12",
  null,
  null,
];

const PORT_PROFILES: number[][] = [
  [80, 443],
  [22],
  [445, 3389],
  [80, 8080],
  [443, 445],
  [3389, 5900],
  [22, 80, 443],
  [21, 23],
  [53, 80],
  [143, 110],
  [],
  [445],
];

function hashString(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

function firstUsableBase(target: string): string {
  // Best-effort: pull an IPv4 prefix out of the target for realistic IPs.
  const m = target.match(/(\d{1,3})\.(\d{1,3})\.(\d{1,3})\./);
  if (m) return `${m[1]}.${m[2]}.${m[3]}`;
  return "192.168.1";
}

function delay(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

export async function mockScan(opts: ScanOptions): Promise<ScanResult> {
  const base = firstUsableBase(opts.target);
  const seed = hashString(opts.target);
  // Deterministic-ish count so re-scanning the same target is stable.
  const count = 6 + (seed % 12);
  const hosts: HostResult[] = [];
  const now = new Date().toISOString();

  for (let i = 0; i < count; i++) {
    const octet = 1 + (((seed >>> (i % 5)) + i * 17) % 253);
    const rng = hashString(`${base}.${octet}.${i}`);
    const knownVendor = rng % 10 !== 0; // ~10% unknown
    const [vendor, ouiPrefix] = VENDORS[rng % VENDORS.length];
    const macTail = [(rng >> 4) & 0xff, (rng >> 12) & 0xff, (rng >> 20) & 0xff]
      .map((b) => b.toString(16).padStart(2, "0").toUpperCase())
      .join(":");
    const ports = PORT_PROFILES[rng % PORT_PROFILES.length];

    hosts.push({
      ip: `${base}.${octet}`,
      hostname: HOSTNAMES[rng % HOSTNAMES.length],
      mac: rng % 7 === 0 ? null : `${ouiPrefix}:${macTail}`,
      vendor: knownVendor && rng % 7 !== 0 ? vendor : null,
      open_ports: ports,
      response_ms: 1 + (rng % 40),
      last_seen: now,
    });
  }

  // De-duplicate by IP and sort numerically.
  const byIp = new Map<string, HostResult>();
  for (const h of hosts) byIp.set(h.ip, h);
  const unique = [...byIp.values()].sort((a, b) => ipToNum(a.ip) - ipToNum(b.ip));

  // Simulate scan time, capped so the demo stays snappy.
  await delay(Math.min(1400, 400 + unique.length * 40));

  return {
    target: opts.target,
    duration_ms: 400 + unique.length * 38,
    scanned: 254,
    hosts: unique,
  };
}

function ipToNum(ip: string): number {
  return ip.split(".").reduce((acc, o) => acc * 256 + parseInt(o, 10), 0);
}

export function mockSave(result: ScanResult): number {
  const id = nextId++;
  store.unshift({ ...result, id, created_at: new Date().toISOString() });
  return id;
}

export function mockList() {
  return store.map((s) => ({
    id: s.id,
    target: s.target,
    created_at: s.created_at,
    duration_ms: s.duration_ms,
    scanned: s.scanned,
    host_count: s.hosts.length,
  }));
}

export function mockGet(id: number) {
  const s = store.find((x) => x.id === id);
  if (!s) throw new Error(`Scan ${id} not found`);
  return {
    id: s.id,
    target: s.target,
    created_at: s.created_at,
    duration_ms: s.duration_ms,
    scanned: s.scanned,
    host_count: s.hosts.length,
    hosts: s.hosts,
  };
}

export function mockDelete(id: number) {
  const idx = store.findIndex((x) => x.id === id);
  if (idx >= 0) store.splice(idx, 1);
}

export function mockLastIps(): string[] {
  if (store.length === 0) return [];
  return store[0].hosts.map((h) => h.ip);
}
