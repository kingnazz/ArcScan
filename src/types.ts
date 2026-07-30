// Shared types, kept in sync with the serde structs in src-tauri/src.
//
// Field names use the Rust spelling (snake_case) deliberately: the payloads
// cross the Tauri boundary unchanged, so renaming them here would mean a
// translation layer with nothing to gain.

export interface HostResult {
  ip: string;
  hostname: string | null;
  mac: string | null;
  vendor: string | null;
  open_ports: number[];
  /** Fastest response of any kind, whole milliseconds. */
  response_ms: number | null;
  /** ICMP round-trip time as reported by the OS ping output. */
  icmp_ms: number | null;
  /** Fastest TCP connection establishment time. */
  tcp_ms: number | null;
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

export type ScanPhase = "probing" | "confirming" | "resolving" | "done" | "cancelled";

export interface ScanStarted {
  scan_id: number;
  target: string;
  profile: string | null;
  total: number;
  port_count: number;
  warning: string | null;
}

export interface ScanProgress {
  scan_id: number;
  done: number;
  total: number;
  found: number;
  phase: ScanPhase;
  elapsed_ms: number;
}

export interface HostEvent {
  scan_id: number;
  host: HostResult;
}

export interface HostRemovedEvent {
  scan_id: number;
  ip: string;
}

export interface ScanResult {
  scan_id: number;
  target: string;
  profile: string | null;
  duration_ms: number;
  /** Addresses the target expands to. */
  scanned: number;
  /** Addresses actually probed; lower than `scanned` when cancelled. */
  probed: number;
  hosts: HostResult[];
  cancelled: boolean;
}

export interface ScanOptions {
  target: string;
  ports: number[];
  timeout_ms: number;
  /** Host concurrency. Named for compatibility with v1.6 saved preferences. */
  concurrency: number;
  tcp_concurrency: number | null;
  ping_concurrency: number | null;
  profile: string | null;
  /** false forces routed behaviour with no local ARP assumptions. */
  arp_assist: boolean | null;
}

export interface ScanPreview {
  total: number;
  port_count: number;
  workload: number;
  warning: string | null;
}

export interface ScanSummary {
  id: number;
  target: string;
  target_key: string;
  profile: string | null;
  created_at: string;
  duration_ms: number;
  scanned: number;
  probed: number;
  host_count: number;
  new_count: number;
  missing_count: number;
  changed_count: number;
  status: "completed" | "cancelled" | string;
  baseline_scan_id: number | null;
}

export type DeviceStatus = "unclassified" | "known" | "trusted" | "watched";

export interface HostDevice {
  ip: string;
  device_id: number | null;
  custom_name: string | null;
  status: DeviceStatus;
  first_seen: string | null;
}

export interface ScanDetail extends ScanSummary {
  hosts: HostResult[];
  devices: HostDevice[];
}

export type IdentitySource = "mac" | "hostname-vendor" | "ip";

export interface Device {
  id: number;
  identity_key: string;
  identity_source: IdentitySource;
  mac: string | null;
  custom_name: string | null;
  hostname: string | null;
  vendor: string | null;
  last_ip: string | null;
  first_seen: string;
  last_seen: string;
  status: DeviceStatus;
  notes: string | null;
  observation_count: number;
}

export interface FieldChange {
  field: string;
  label: string;
  from: string | null;
  to: string | null;
  added_ports: number[];
  removed_ports: number[];
}

export type ChangeKind = "new" | "returned" | "missing" | "changed";

export interface DeviceDiff {
  kind: ChangeKind;
  device_id: number | null;
  name: string;
  ip: string;
  mac: string | null;
  vendor: string | null;
  hostname: string | null;
  last_seen: string | null;
  fields: FieldChange[];
}

export interface ScanComparison {
  scan_id: number;
  baseline_scan_id: number | null;
  baseline_created_at: string | null;
  baseline_target: string | null;
  /** Set when no compatible earlier scan exists. */
  reason: string | null;
  added: DeviceDiff[];
  removed: DeviceDiff[];
  changed: DeviceDiff[];
}

export interface SavedScan {
  scan_id: number;
  comparison: ScanComparison;
}

export interface DeviceObservation {
  scan_id: number;
  scan_target: string;
  observed_at: string;
  ip: string;
  hostname: string | null;
  vendor: string | null;
  open_ports: number[];
  response_ms: number | null;
  icmp_ms: number | null;
  tcp_ms: number | null;
  ttl: number | null;
  os_guess: string | null;
}

export interface DeviceDetail {
  device: Device;
  observations: DeviceObservation[];
  previous_ips: string[];
  recent_changes: FieldChange[];
}

export interface ServiceInfo {
  port: number;
  name: string;
  sensitive: boolean;
}

/** Total reported changes, which is what the change badges count. */
export function changeCount(comparison: ScanComparison | null): number {
  if (!comparison) return 0;
  return comparison.added.length + comparison.removed.length + comparison.changed.length;
}
