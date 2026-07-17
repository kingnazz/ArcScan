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
  21: "FTP",
  22: "SSH",
  23: "Telnet",
  25: "SMTP",
  53: "DNS",
  80: "HTTP",
  110: "POP3",
  139: "NetBIOS",
  143: "IMAP",
  443: "HTTPS",
  445: "SMB",
  3306: "MySQL",
  3389: "RDP",
  5432: "PostgreSQL",
  5900: "VNC",
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

// Parse a port spec supporting single ports and ranges, e.g.
// "22, 80, 443" or "1-1024" or "80,443,8000-8100". De-duplicated, sorted,
// and capped so a huge range can't lock up the scan.
export function parsePorts(input: string, cap = 2048): number[] {
  const valid = (n: number) => Number.isInteger(n) && n > 0 && n <= 65535;
  const set = new Set<number>();
  for (const tokenRaw of input.split(/[,\s]+/)) {
    const token = tokenRaw.trim();
    if (!token) continue;
    const range = token.match(/^(\d+)-(\d+)$/);
    if (range) {
      let a = parseInt(range[1], 10);
      let b = parseInt(range[2], 10);
      if (a > b) [a, b] = [b, a];
      if (!valid(a) || !valid(b)) continue;
      for (let p = a; p <= b && set.size < cap; p++) set.add(p);
    } else {
      const n = parseInt(token, 10);
      if (valid(n)) set.add(n);
    }
    if (set.size >= cap) break;
  }
  return [...set].sort((a, b) => a - b);
}
