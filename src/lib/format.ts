// Formatting helpers and the service-name lookup.

import type { ServiceInfo } from "../types";

export function ipToNum(ip: string): number {
  const parts = ip.split(".");
  if (parts.length !== 4) return 0;
  return parts.reduce((acc, o) => acc * 256 + (Number.parseInt(o, 10) || 0), 0);
}

export function formatRelative(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return iso;
  const s = Math.round((Date.now() - then) / 1000);
  if (s < 60) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.round(h / 24);
  if (d < 30) return `${d}d ago`;
  const mo = Math.round(d / 30);
  if (mo < 12) return `${mo}mo ago`;
  return `${Math.round(mo / 12)}y ago`;
}

export function formatDateTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  const minutes = Math.floor(ms / 60_000);
  const seconds = Math.round((ms % 60_000) / 1000);
  return `${minutes}m ${seconds.toString().padStart(2, "0")}s`;
}

/**
 * Latency for display. Sub-millisecond LAN responses keep two decimals so a
 * switched wired link does not read the same as a slow Wi-Fi one; anything above
 * 10 ms is rounded, because the extra precision is noise.
 */
export function formatLatency(ms: number | null | undefined): string | null {
  if (ms == null || !Number.isFinite(ms)) return null;
  if (ms < 1) return `${ms.toFixed(2)} ms`;
  if (ms < 10) return `${ms.toFixed(1)} ms`;
  return `${Math.round(ms)} ms`;
}

export function formatCount(n: number): string {
  return n.toLocaleString();
}

// ---------------------------------------------------------------------------
// Services
//
// The backend owns the service table and hands it over once at startup, so the
// UI has no second copy to keep in sync. Until it arrives (or in the browser
// demo) this fallback covers the ports the default profile probes, which is
// enough that the table never shows a bare number where a name belongs.
// ---------------------------------------------------------------------------

const FALLBACK_SERVICES: ServiceInfo[] = [
  { port: 21, name: "FTP", sensitive: true },
  { port: 22, name: "SSH", sensitive: false },
  { port: 23, name: "Telnet", sensitive: true },
  { port: 53, name: "DNS", sensitive: false },
  { port: 80, name: "HTTP", sensitive: false },
  { port: 110, name: "POP3", sensitive: false },
  { port: 139, name: "NetBIOS", sensitive: true },
  { port: 143, name: "IMAP", sensitive: false },
  { port: 443, name: "HTTPS", sensitive: false },
  { port: 445, name: "SMB", sensitive: true },
  { port: 3389, name: "RDP", sensitive: true },
  { port: 5900, name: "VNC", sensitive: true },
  { port: 8080, name: "HTTP-alt", sensitive: false },
  { port: 8443, name: "HTTPS-alt", sensitive: false },
];

let serviceMap = new Map<number, ServiceInfo>(FALLBACK_SERVICES.map((s) => [s.port, s]));

/** Install the catalog fetched from the backend. */
export function setServiceCatalog(services: ServiceInfo[]): void {
  if (services.length === 0) return;
  serviceMap = new Map(services.map((s) => [s.port, s]));
}

export function serviceInfo(port: number): ServiceInfo | undefined {
  return serviceMap.get(port);
}

/** Service name for a port, or the number when none is known. */
export function serviceLabel(port: number): string {
  return serviceMap.get(port)?.name ?? String(port);
}

/** "HTTPS · 443", the form used in the table and in change summaries. */
export function serviceWithPort(port: number): string {
  const name = serviceMap.get(port)?.name;
  return name ? `${name} · ${port}` : String(port);
}

export function isSensitivePort(port: number): boolean {
  return serviceMap.get(port)?.sensitive ?? false;
}

/** The port to use for a device's web interface, preferring HTTPS. */
export function webPort(ports: number[]): number | null {
  for (const p of [443, 8443, 80, 8080, 8000, 8081]) {
    if (ports.includes(p)) return p;
  }
  return null;
}

/**
 * Parse a port specification for immediate feedback while typing. The backend
 * re-parses every spec before a scan runs and is the authority; this exists only
 * so the field can show a count and an error without a round trip.
 */
export function parsePorts(input: string, cap = 2048): { ports: number[]; error: string | null } {
  const text = input.trim();
  if (!text) return { ports: [], error: null };
  const set = new Set<number>();
  const valid = (n: number) => Number.isInteger(n) && n >= 1 && n <= 65535;

  for (const raw of text.split(/[,\s]+/)) {
    const token = raw.trim();
    if (!token) continue;
    const range = token.match(/^(\d+)-(\d+)$/);
    if (range) {
      let a = Number.parseInt(range[1], 10);
      let b = Number.parseInt(range[2], 10);
      if (a > b) [a, b] = [b, a];
      if (!valid(a) || !valid(b)) {
        return { ports: [], error: `"${token}" is out of range. Ports are 1 to 65535.` };
      }
      if (set.size + (b - a + 1) > cap) {
        return {
          ports: [],
          error: `"${token}" takes the selection past the ${cap.toLocaleString()} port limit.`,
        };
      }
      for (let p = a; p <= b; p++) set.add(p);
      continue;
    }
    const n = Number.parseInt(token, 10);
    if (!/^\d+$/.test(token)) {
      return { ports: [], error: `"${token}" is not a port number.` };
    }
    if (!valid(n)) {
      // Same wording as the backend, so the operator never sees two phrasings of
      // the same rule depending on which side rejected it.
      return { ports: [], error: `"${token}" is out of range. Ports are 1 to 65535.` };
    }
    set.add(n);
    if (set.size > cap) {
      return { ports: [], error: `More than ${cap.toLocaleString()} ports selected.` };
    }
  }

  if (set.size === 0) return { ports: [], error: "No valid ports in the list." };
  return { ports: [...set].sort((a, b) => a - b), error: null };
}

/** The phase label shown in the progress strip. */
export function phaseLabel(phase: string): string {
  switch (phase) {
    case "probing":
      return "Probing addresses";
    case "confirming":
      return "Confirming quiet devices";
    case "discovering":
      return "Discovering local services";
    case "describing":
      return "Reading device descriptions";
    case "classifying":
      return "Classifying devices";
    case "resolving":
      return "Resolving names and vendors";
    case "done":
      return "Finished";
    case "cancelled":
      return "Stopped";
    default:
      return "Scanning";
  }
}
