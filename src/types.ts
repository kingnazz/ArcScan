// Shared types — kept in sync with the Rust serde structs in src-tauri/src.

export interface HostResult {
  ip: string;
  hostname: string | null;
  mac: string | null;
  vendor: string | null;
  open_ports: number[];
  response_ms: number | null;
  ttl: number | null;
  os_guess: string | null;
  last_seen: string;
}

export interface LocalNetwork {
  interface: string;
  ip: string;
  prefix: number;
  cidr: string;
  is_private: boolean;
}

export type ExportFormat = "csv" | "json" | "xml";

export interface ScanResult {
  target: string;
  duration_ms: number;
  scanned: number;
  hosts: HostResult[];
}

export interface ScanOptions {
  target: string;
  ports: number[];
  timeout_ms: number;
  concurrency: number;
}

export interface ScanSummary {
  id: number;
  target: string;
  created_at: string;
  duration_ms: number;
  scanned: number;
  host_count: number;
}

export interface ScanDetail extends ScanSummary {
  hosts: HostResult[];
}

export const DEFAULT_PORTS = [
  21, 22, 23, 53, 80, 110, 139, 143, 443, 445, 3389, 5900, 8080, 8443,
];

export interface DashboardStats {
  total: number;
  unknown: number;
  openRdp: number;
  openSmb: number;
  newDevices: number;
}
