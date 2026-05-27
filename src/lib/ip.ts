// IP range / CIDR parsing and RFC1918 validation for the frontend.
// The Rust backend performs the authoritative validation again; this exists
// to give the operator immediate feedback before a scan is dispatched.

function ipToInt(ip: string): number | null {
  const parts = ip.trim().split(".");
  if (parts.length !== 4) return null;
  let value = 0;
  for (const part of parts) {
    if (!/^\d{1,3}$/.test(part)) return null;
    const n = Number(part);
    if (n < 0 || n > 255) return null;
    value = value * 256 + n;
  }
  // Force unsigned 32-bit.
  return value >>> 0;
}

export function intToIp(value: number): string {
  return [
    (value >>> 24) & 0xff,
    (value >>> 16) & 0xff,
    (value >>> 8) & 0xff,
    value & 0xff,
  ].join(".");
}

const PRIVATE_RANGES: Array<[number, number]> = [
  // 10.0.0.0/8
  [ipToInt("10.0.0.0")!, ipToInt("10.255.255.255")!],
  // 172.16.0.0/12
  [ipToInt("172.16.0.0")!, ipToInt("172.31.255.255")!],
  // 192.168.0.0/16
  [ipToInt("192.168.0.0")!, ipToInt("192.168.255.255")!],
];

export function isPrivateIp(ip: string): boolean {
  const n = ipToInt(ip);
  if (n === null) return false;
  return PRIVATE_RANGES.some(([lo, hi]) => n >= lo && n <= hi);
}

export interface ParsedTarget {
  ok: boolean;
  /** Number of addresses the target expands to. */
  count: number;
  /** True when every address falls inside an RFC1918 private range. */
  allPrivate: boolean;
  error?: string;
  first?: string;
  last?: string;
}

const MAX_HOSTS = 65536;

/**
 * Parse a CIDR block ("192.168.1.0/24") or an inclusive dashed range
 * ("192.168.1.10-192.168.1.50" or "192.168.1.10-50") into a summary.
 */
export function parseTarget(input: string): ParsedTarget {
  const target = input.trim();
  if (!target) return { ok: false, count: 0, allPrivate: false, error: "Enter an IP range or CIDR." };

  let lo: number | null = null;
  let hi: number | null = null;

  if (target.includes("/")) {
    const [addr, prefixStr] = target.split("/");
    const base = ipToInt(addr);
    const prefix = Number(prefixStr);
    if (base === null || !Number.isInteger(prefix) || prefix < 0 || prefix > 32) {
      return { ok: false, count: 0, allPrivate: false, error: "Invalid CIDR notation." };
    }
    const mask = prefix === 0 ? 0 : (0xffffffff << (32 - prefix)) >>> 0;
    lo = (base & mask) >>> 0;
    hi = (lo | (~mask >>> 0)) >>> 0;
  } else if (target.includes("-")) {
    const [startStr, endStr] = target.split("-").map((s) => s.trim());
    const start = ipToInt(startStr);
    if (start === null) {
      return { ok: false, count: 0, allPrivate: false, error: "Invalid start address." };
    }
    let end: number | null;
    if (endStr.includes(".")) {
      end = ipToInt(endStr);
    } else if (/^\d{1,3}$/.test(endStr)) {
      // Shorthand: replace the final octet.
      end = ((start & 0xffffff00) >>> 0) | Number(endStr);
    } else {
      end = null;
    }
    if (end === null) {
      return { ok: false, count: 0, allPrivate: false, error: "Invalid end address." };
    }
    lo = Math.min(start, end) >>> 0;
    hi = Math.max(start, end) >>> 0;
  } else {
    const single = ipToInt(target);
    if (single === null) {
      return { ok: false, count: 0, allPrivate: false, error: "Invalid IP address." };
    }
    lo = single;
    hi = single;
  }

  const count = hi - lo + 1;
  if (count > MAX_HOSTS) {
    return {
      ok: false,
      count,
      allPrivate: false,
      error: `Range too large (${count.toLocaleString()} hosts). Limit is ${MAX_HOSTS.toLocaleString()}.`,
    };
  }

  const firstIp = intToIp(lo);
  const lastIp = intToIp(hi);
  const allPrivate = isPrivateIp(firstIp) && isPrivateIp(lastIp);

  return { ok: true, count, allPrivate, first: firstIp, last: lastIp };
}
