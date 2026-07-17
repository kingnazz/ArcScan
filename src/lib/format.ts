// Small formatting + service helpers shared across components.

export function ipToNum(ip: string): number {
  const parts = ip.split(".");
  if (parts.length !== 4) return 0;
  return parts.reduce((acc, o) => acc * 256 + (parseInt(o, 10) || 0), 0);
}

export function formatRelative(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return iso;
  const diff = Date.now() - then;
  const s = Math.round(diff / 1000);
  if (s < 60) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.round(h / 24);
  return `${d}d ago`;
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
  if (ms < 1000) return `${ms} ms`;
  return `${(ms / 1000).toFixed(1)} s`;
}

// Well-known port → service label used for the service badges and quick actions.
export const PORT_SERVICES: Record<number, string> = {
  22: "SSH",
  80: "HTTP",
  443: "HTTPS",
  445: "SMB",
  3389: "RDP",
  8080: "HTTP-alt",
  8443: "HTTPS-alt",
};

export function serviceLabel(port: number): string {
  return PORT_SERVICES[port] ?? String(port);
}

export function hasWeb(ports: number[]): number | null {
  for (const p of [80, 443, 8080, 8443]) {
    if (ports.includes(p)) return p;
  }
  return null;
}

export function parsePorts(input: string): number[] {
  return input
    .split(/[,\s]+/)
    .map((s) => parseInt(s.trim(), 10))
    .filter((n) => Number.isInteger(n) && n > 0 && n <= 65535);
}
