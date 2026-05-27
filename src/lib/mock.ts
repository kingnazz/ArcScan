// Mock scanner used when ArcScan runs outside the Tauri runtime (e.g. plain
// `vite dev` in a browser). It lets the UI be developed and demoed without the
// Rust backend, and is never bundled into the desktop build path.

import type { Host, ScanOptions, ScanProgress, ScanResult } from "../types";
import { PORT_SERVICES } from "../types";
import { intToIp, parseTarget } from "./ip";

const VENDORS = [
  ["Ubiquiti Inc", "fc:ec:da"],
  ["Apple, Inc.", "a4:83:e7"],
  ["Dell Inc.", "00:14:22"],
  ["Hewlett Packard", "00:1b:78"],
  ["Cisco Systems", "00:1a:a1"],
  ["Synology", "00:11:32"],
  ["Raspberry Pi", "dc:a6:32"],
  ["Intel Corporate", "94:c6:91"],
] as const;

const HOSTNAMES = [
  "fileserver-01",
  "dc-primary",
  "reception-pc",
  "nas-backup",
  "printer-hp-2nd",
  "ap-lobby",
  "switch-core",
  "ws-accounting",
  null,
  null,
];

function pick<T>(arr: readonly T[], seed: number): T {
  return arr[seed % arr.length];
}

function randomMac(prefix: string, seed: number): string {
  const tail = [(seed * 7) % 256, (seed * 13) % 256, (seed * 29) % 256]
    .map((n) => n.toString(16).padStart(2, "0"))
    .join(":");
  return `${prefix}:${tail}`;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export async function mockScan(
  options: ScanOptions,
  onProgress: (p: ScanProgress) => void,
  onHost: (h: Host) => void
): Promise<ScanResult> {
  const parsed = parseTarget(options.target);
  const startedAt = new Date().toISOString();
  const total = parsed.ok ? Math.min(parsed.count, 256) : 0;

  if (!parsed.first) {
    return {
      scanId: null,
      target: options.target,
      startedAt,
      finishedAt: new Date().toISOString(),
      hosts: [],
      totalScanned: 0,
    };
  }

  const baseInt =
    (parseInt(parsed.first.split(".").map((o) => Number(o).toString(16).padStart(2, "0")).join(""), 16)) >>> 0;

  const hosts: Host[] = [];
  let found = 0;

  for (let i = 0; i < total; i++) {
    await sleep(8);
    const ip = intToIp((baseInt + i) >>> 0);
    onProgress({ scanned: i + 1, total, found });

    // ~30% of addresses are "alive" in the mock.
    const alive = (i * 2654435761) % 10 < 3;
    if (!alive) continue;

    const [vendor, prefix] = pick(VENDORS, i);
    const openPorts = Object.keys(PORT_SERVICES)
      .map(Number)
      .filter((p) => (i + p) % 3 === 0)
      .map((port) => ({ port, service: PORT_SERVICES[port] }));

    const host: Host = {
      ip,
      hostname: pick(HOSTNAMES, i) ?? null,
      mac: randomMac(prefix, i),
      vendor,
      openPorts,
      responseMs: Math.round((((i * 9301 + 49297) % 233280) / 233280) * 40 + 1),
      status: "up",
      lastSeen: new Date().toISOString(),
      isNew: i % 5 === 0,
    };
    found++;
    hosts.push(host);
    onHost(host);
    onProgress({ scanned: i + 1, total, found });
  }

  return {
    scanId: Math.floor(Math.random() * 1000),
    target: options.target,
    startedAt,
    finishedAt: new Date().toISOString(),
    hosts,
    totalScanned: total,
  };
}
