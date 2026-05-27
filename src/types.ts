// Shared types between the React frontend and the Rust backend.
// These mirror the serde-serialized structs in `src-tauri/src/scanner.rs`
// and `src-tauri/src/db.rs`.

export type HostStatus = "up" | "down";

export interface PortResult {
  port: number;
  /** Friendly service name, e.g. "HTTPS", "RDP". */
  service: string;
}

export interface Host {
  ip: string;
  hostname: string | null;
  mac: string | null;
  vendor: string | null;
  openPorts: PortResult[];
  /** Round-trip time in milliseconds, if measured. */
  responseMs: number | null;
  status: HostStatus;
  /** ISO-8601 timestamp of when the host was last observed. */
  lastSeen: string;
  /** True when this host was not present in the previous saved scan. */
  isNew: boolean;
}

export interface ScanProgress {
  scanned: number;
  total: number;
  found: number;
}

export interface ScanResult {
  /** Database id of the persisted scan, if saved. */
  scanId: number | null;
  target: string;
  startedAt: string;
  finishedAt: string;
  hosts: Host[];
  totalScanned: number;
}

export interface ScanSummary {
  id: number;
  target: string;
  startedAt: string;
  finishedAt: string;
  hostsUp: number;
  totalScanned: number;
}

export interface ScanOptions {
  /** IP range or CIDR, e.g. "192.168.1.0/24" or "10.0.0.1-10.0.0.50". */
  target: string;
  /** Per-host connection/ping timeout in milliseconds. */
  timeoutMs: number;
  /** Maximum concurrent host probes. */
  concurrency: number;
  /** TCP ports to probe as an ICMP fallback / service detector. */
  ports: number[];
  /** When false, scanning non-RFC1918 targets is rejected by the backend. */
  allowPublic: boolean;
  /** Explicit operator acknowledgement that they are authorized to scan. */
  authorized: boolean;
}

/** The canonical service ports ArcScan probes, per the MVP spec. */
export const DEFAULT_PORTS = [22, 80, 443, 445, 3389, 8080];

export const PORT_SERVICES: Record<number, string> = {
  22: "SSH",
  80: "HTTP",
  443: "HTTPS",
  445: "SMB",
  3389: "RDP",
  8080: "HTTP-Alt",
};
