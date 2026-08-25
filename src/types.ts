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
  /** What local discovery learned, when it ran. Absent for remote scans. */
  discovery?: HostDiscovery | null;
}

/** The fourteen types ArcScan is prepared to name. */
export type DeviceType =
  | "router"
  | "printer"
  | "computer"
  | "phone"
  | "tablet"
  | "television"
  | "media_device"
  | "camera"
  | "nas"
  | "game_console"
  | "smart_home"
  | "network_equipment"
  | "speaker"
  | "unknown";

/**
 * How sure ArcScan is, as a word.
 *
 * Never a number: there is no sense in which a printer service is 0.7 of a
 * printer, and a score invites arithmetic that is not justified.
 */
export type Confidence = "high" | "medium" | "low" | "unknown";

/** Where a detected fact came from. */
export type DiscoverySource =
  | "user"
  | "ssdp"
  | "mdns"
  | "reverse_dns"
  | "arp_vendor"
  | "tcp_service"
  | "scan_observation";

/** What one scan's discovery pass managed. */
export type DiscoveryMode = "full" | "partial" | "none";

/**
 * How well a scan's discovery pass went, for a person reading History.
 *
 * Deliberately separate from `DiscoveryMode`, which gates whether two scans may
 * be compared and whose meaning may not move.
 */
export type DiscoveryQuality = "complete" | "limited" | "skipped" | "interrupted";

/**
 * How current a piece of discovery evidence is, counted in qualifying scans
 * rather than in days. ArcScan only learns when it runs, so the calendar is
 * shown as context and never used to decide anything.
 */
export type Freshness = "current" | "aging" | "stale";

/** Who decided the device type shown on screen. */
export type TypeSource = "user" | "automatic";

/** Discovery facts attached to one observation. */
export interface HostDiscovery {
  detected_name: string | null;
  name_source: string | null;
  device_type: string | null;
  type_confidence: string | null;
  type_evidence: string[];
  type_conflicts: string[];
  manufacturer: string | null;
  model_name: string | null;
  model_number: string | null;
  serial_number: string | null;
  mdns_hostname: string | null;
  ssdp_friendly_name: string | null;
  services: string[];
  sources: string[];
  alternate_names: string[];
  /** Learned from mDNS. Supplemental display only — ArcScan scans IPv4. */
  ipv6_addresses: string[];
  presentation_url: string | null;
  last_discovered_at: string | null;
}

/** The discovery fields the Inventory table, search and export use. */
export interface InventoryDiscovery {
  detected_name: string | null;
  /**
   * What ArcScan detected. The type shown on screen is the row's
   * `user_device_type` when there is one and this otherwise; `resolveType` in
   * `lib/effectiveType` is the only place that decides between them.
   */
  device_type: string;
  /** The detected confidence, already reduced where its evidence is stale. */
  type_confidence: string;
  manufacturer: string | null;
  model_name: string | null;
  services: string[];
  sources: string[];
  last_discovered_at: string | null;
  /** How current the freshest claim behind this record is. */
  evidence_freshness: Freshness | string;
}

/** One stored claim about a device. */
export interface DiscoveryEvidenceRow {
  source: string;
  kind: string;
  key: string;
  value: string;
  confidence: string;
  first_seen: string;
  last_seen: string;
  /** `current`, `aging` or `stale`. */
  freshness: Freshness | string;
  /** Consecutive qualifying discovery scans that did not re-observe this claim. */
  misses: number;
}

/** The full discovery record for one device, as the drawer shows it. */
export interface DeviceDiscovery {
  detected_name: string | null;
  name_source: string | null;
  device_type: string;
  type_confidence: string;
  type_evidence: string[];
  type_conflicts: string[];
  manufacturer: string | null;
  model_name: string | null;
  model_number: string | null;
  serial_number: string | null;
  mdns_hostname: string | null;
  ssdp_friendly_name: string | null;
  services: string[];
  sources: string[];
  alternate_names: string[];
  ipv6_addresses: string[];
  presentation_url: string | null;
  first_discovered_at: string | null;
  last_discovered_at: string | null;
  /** The durable record, distinct from the per-scan observation history. */
  evidence: DiscoveryEvidenceRow[];
  /** How current the freshest claim behind this record is. */
  evidence_freshness: Freshness | string;
  /**
   * The confidence the classifier reached before any reduction for stale
   * evidence, so the drawer can explain a reduction rather than only show it.
   */
  raw_type_confidence: string;
}

/** What a scan's discovery pass did, as recorded with the scan. */
export interface DiscoveryReport {
  mdns_attempted: boolean;
  ssdp_attempted: boolean;
  mdns_responses: number;
  ssdp_responses: number;
  descriptions_fetched: number;
  descriptions_rejected: number;
  description_notes: string[];
  devices_enriched: number;
  duration_ms: number;
  skip_reason: string | null;
  interrupted: boolean;
  /** True when a socket could not be opened. Observed, never inferred. */
  mdns_socket_failed?: boolean;
  ssdp_socket_failed?: boolean;
  /** True when a response cap stopped the listening while the link was talking. */
  mdns_capped?: boolean;
  ssdp_capped?: boolean;
  /** True when the description budget ran out with documents still queued. */
  descriptions_capped?: boolean;
}

/** One device type, as the backend defines it. */
export interface DeviceTypeInfo {
  id: string;
  label: string;
}

/** Which parts of local discovery a scan should run. */
export interface DiscoveryOptions {
  enabled: boolean;
  mdns: boolean;
  ssdp: boolean;
  descriptions: boolean;
}

export interface LocalNetwork {
  interface: string;
  ip: string;
  prefix: number;
  cidr: string;
  is_private: boolean;
}

export type ExportFormat = "csv" | "json" | "xml";

export type ScanPhase =
  | "probing"
  | "confirming"
  | "discovering"
  | "describing"
  | "resolving"
  | "classifying"
  | "done"
  | "cancelled";

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
  /** The sanitized port set the scan actually probed. */
  ports: number[];
  /** The ARP-assist strategy the scan ran with. */
  arp_assist: boolean | null;
  /** Performance tuning, recorded for transparency only. */
  execution?: ExecutionSettings | null;
  /** Evidence about which physical network was scanned. */
  scope_hint?: ScopeHint | null;
  /** What the discovery pass did, or why it did not run. */
  discovery?: DiscoveryReport | null;
}

export interface ExecutionSettings {
  timeout_ms: number;
  host_concurrency: number;
  tcp_concurrency: number;
  ping_concurrency: number;
}

export interface ScopeHint {
  local_network: string | null;
  gateway_ip: string | null;
  gateway_mac: string | null;
  interface: string | null;
}

/** One persistent network scope: a physical network as ArcScan understands it. */
export interface NetworkScope {
  id: number;
  stable_key: string;
  display_name: string;
  canonical_target: string | null;
  gateway_mac: string | null;
  interface_hint: string | null;
  created_at: string;
  updated_at: string;
  device_count: number;
  scan_count: number;
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
  /** Which parts of local discovery to run. Absent means all of them. */
  discovery?: DiscoveryOptions | null;
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
  /** The network scope this scan belongs to. */
  network_scope_id: number | null;
  /** The scope's display name, joined in by the backend. */
  scope_name: string | null;
  /** Ports-and-discovery-mode signature; scans compare only when it matches. */
  coverage_key: string;
  /** What the scan's local-discovery pass managed. Not part of coverage. */
  discovery_mode: DiscoveryMode | string;
  /** The scan's DiscoveryReport as stored JSON, or null. */
  discovery_summary: string | null;
  /** How well the discovery pass went, derived by the backend. */
  discovery_quality?: DiscoveryQuality | string;
  /**
   * The one thing ArcScan observed that made the pass less than complete, or
   * null. Never a diagnosis — ArcScan cannot see a firewall and never blames
   * one.
   */
  discovery_quality_reason?: string | null;
}

export type DeviceStatus = "unclassified" | "known" | "trusted" | "watched" | "ignored";

/**
 * What the latest completed scan says about a device.
 *
 * ArcScan does not watch a network continuously, so these three values are the
 * only honest ones. The rules are implemented and documented in
 * `src-tauri/src/inventory.rs`; in short, presence is decided only from a
 * network's most recent scan that both completed and recorded which ports it
 * checked, and a device is only Missing when that scan looked where it used to
 * be and did not find it.
 */
export type PresenceState = "present" | "missing" | "unknown";

export type ChangeType =
  | "device_added"
  | "device_returned"
  | "device_missing"
  | "ip_changed"
  | "hostname_changed"
  | "vendor_changed"
  | "os_changed"
  | "mac_changed"
  | "ports_changed"
  | "detected_name_changed"
  | "device_type_changed"
  | "service_appeared"
  | "service_disappeared"
  | "model_changed";

export type ChangeState = "unreviewed" | "acknowledged" | "ignored";

/** One row of the persistent Inventory. */
export interface InventoryRow {
  device_id: number;
  network_scope_id: number | null;
  network_name: string | null;
  identity_source: IdentitySource;
  display_name: string;
  custom_name: string | null;
  hostname: string | null;
  current_ip: string | null;
  previous_ips: string[];
  mac: string | null;
  vendor: string | null;
  os_guess: string | null;
  status: DeviceStatus;
  presence: PresenceState;
  first_seen: string;
  last_seen: string;
  last_completed_scan_id: number | null;
  last_completed_scan_at: string | null;
  observation_count: number;
  open_ports: number[];
  /** True when the device carries notes. */
  notes_present: boolean;
  /** The opening of the note, so search can reach it without loading it all. */
  notes_excerpt: string | null;
  latest_response_ms: number | null;
  latest_icmp_ms: number | null;
  latest_tcp_ms: number | null;
  /** What local discovery established, if a discovery-capable scan reached it. */
  discovery?: InventoryDiscovery | null;
  /**
   * The operator's device-type correction, or null for Auto.
   *
   * On the row rather than inside `discovery`, because a device discovery has
   * never reached still has a type the operator may have corrected.
   */
  user_device_type?: string | null;
}

/** A network as the Inventory and Changes filters offer it. */
export interface NetworkOption {
  id: number;
  name: string;
  device_count: number;
}

export interface InventorySummary {
  rows: InventoryRow[];
  networks: NetworkOption[];
  present: number;
  missing: number;
  unknown: number;
  /** True when no completed scan anywhere can decide presence. */
  needs_completed_scan: boolean;
}

/** One persisted change, as the Changes inbox shows it. */
export interface ChangeEvent {
  id: number;
  event_key: string;
  scan_id: number | null;
  baseline_scan_id: number | null;
  network_scope_id: number | null;
  network_name: string | null;
  device_id: number | null;
  device_label: string;
  ip: string | null;
  mac: string | null;
  vendor: string | null;
  change_type: ChangeType;
  old_value: string | null;
  new_value: string | null;
  opened_ports: number[];
  closed_ports: number[];
  state: ChangeState;
  created_at: string;
  scan_at: string | null;
  baseline_at: string | null;
  acknowledged_at: string | null;
  device_status: DeviceStatus | null;
}

export interface ChangeFeed {
  events: ChangeEvent[];
  unreviewed: number;
  total: number;
  truncated: boolean;
  /** Newest scan present when the database was upgraded to the v1.8 schema. */
  starts_after_scan_id: number;
}

/** What a bulk action actually managed to do. */
export interface BulkOutcome {
  updated: number;
  /** Ids that no longer existed. The rest still committed. */
  missing: number[];
}

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
  /** The network scope this device belongs to; identity never crosses it. */
  network_scope_id: number | null;
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
  /**
   * The operator's device-type correction, or null for Auto.
   *
   * An operator label alongside the name, the status and the notes. Nothing
   * about how devices are matched, scoped or compared reads it.
   */
  user_device_type?: string | null;
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
  /** Persisted change events for this device, newest first. */
  events: ChangeEvent[];
  network_name: string | null;
  presence: PresenceState;
  /** The full discovery record, with the evidence behind it. */
  discovery?: DeviceDiscovery | null;
}

export interface ServiceInfo {
  port: number;
  name: string;
  sensitive: boolean;
}
