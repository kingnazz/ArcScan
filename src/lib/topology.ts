// Topology discovery types and helpers.
//
// The Rust engine serializes with serde camelCase so this payload matches
// GitHub issue #42. That is a deliberate exception to the snake_case used by
// the rest of ArcScan's IPC: topology is a standalone contract, not a rewrite
// of the inventory exporter.

import type { InventoryRow } from "../types";
import type { DeviceRow } from "./live";

export type TopologyConfidence = "confirmed" | "strong" | "inferred";

export interface PoeInfo {
  enabled: boolean;
  watts?: number | null;
}

export interface UnresolvedNode {
  id: string;
  chassisId?: string | null;
  sysName?: string | null;
  managementAddress?: string | null;
  reason: string;
  source: string;
}

export interface TopologyConnection {
  fromDeviceId?: number | null;
  toDeviceId?: number | null;
  fromUnresolvedId?: string | null;
  toUnresolvedId?: string | null;
  fromPort?: string | null;
  toPort?: string | null;
  kind: string;
  protocol: string;
  confidence: TopologyConfidence;
  speedMbps?: number | null;
  vlan?: string | null;
  nativeVlan?: number | null;
  taggedVlans?: number[];
  poe?: PoeInfo | null;
  evidence: string[];
}

export interface TopologySnapshot {
  capturedAt: string;
  connections: TopologyConnection[];
  unknownNodes?: UnresolvedNode[];
}

export interface TopologyDeviceFailure {
  ip: string;
  reason: string;
}

export interface TopologySummary {
  devicesQueried: number;
  devicesResponded: number;
  devicesFailed: number;
  confirmed: number;
  strong: number;
  inferred: number;
  unknownNodes: number;
  durationMs: number;
  cancelled: boolean;
  timedOut: boolean;
  failures: TopologyDeviceFailure[];
}

export interface TopologyResult {
  snapshot: TopologySnapshot;
  summary: TopologySummary;
}

export interface TopologyTarget {
  ip: string;
  mac?: string | null;
  /** The exact local Inventory row id. Topology never mints its own ids. */
  deviceId: number;
  hostname?: string | null;
  detectedName?: string | null;
}

export interface TopologyRequest {
  targets: TopologyTarget[];
  timeoutMs?: number | null;
  concurrency?: number | null;
  networkName?: string | null;
  scanId?: number | null;
}

export type SnmpVersion = "v2c" | "v3";

export interface CredentialInput {
  version: SnmpVersion;
  community?: string | null;
  username?: string | null;
  authProtocol?: string | null;
  authPassword?: string | null;
  privProtocol?: string | null;
  privPassword?: string | null;
  context?: string | null;
}

export interface CredentialStatus {
  configured: boolean;
  version: string | null;
  username: string | null;
  authProtocol: string | null;
  privProtocol: string | null;
  sessionOnly: boolean;
}

export const EMPTY_CREDENTIAL_STATUS: CredentialStatus = {
  configured: false,
  version: null,
  username: null,
  authProtocol: null,
  privProtocol: null,
  sessionOnly: true,
};

export const SNMP_AUTH_PROTOCOLS = ["sha256", "sha1", "sha512", "sha384", "sha224", "md5"] as const;
export const SNMP_PRIV_PROTOCOLS = ["aes128", "aes256", "aes192", "des"] as const;

export function emptyCredentialInput(version: SnmpVersion = "v2c"): CredentialInput {
  return {
    version,
    community: "",
    username: "",
    authProtocol: version === "v3" ? "sha256" : "",
    authPassword: "",
    privProtocol: version === "v3" ? "aes128" : "",
    privPassword: "",
    context: "",
  };
}

/** Client-side completeness check. The backend is the authority. */
export function credentialInputError(input: CredentialInput): string | null {
  if (input.version === "v2c") {
    if (!input.community?.trim()) {
      return "Enter an SNMP community string. ArcScan never tries public, private or any other default.";
    }
    return null;
  }
  if (!input.username?.trim()) return "Enter an SNMPv3 username.";
  if (!input.authProtocol?.trim()) {
    return "ArcScan does not send SNMPv3 with noAuthNoPriv. Choose authentication, and privacy when the device requires it.";
  }
  if (!input.authPassword?.trim()) return "Enter the SNMPv3 authentication password.";
  if (input.privProtocol?.trim() && !input.privPassword?.trim()) {
    return "Enter the SNMPv3 privacy password.";
  }
  return null;
}

export function targetsFromScanRows(rows: DeviceRow[]): TopologyTarget[] {
  return rows
    // A row receives its local device id when the completed scan is persisted.
    // Waiting for that id is what makes every topology endpoint joinable to the
    // Inventory JSON sent to ArcAtlas; an IP, hostname or array index must never
    // become a substitute identity.
    .filter((row): row is DeviceRow & { device_id: number } =>
      row.host.ip.trim().length > 0 && row.device_id != null,
    )
    .map((row) => ({
      ip: row.host.ip,
      mac: row.host.mac,
      deviceId: row.device_id,
      hostname: row.host.hostname,
      detectedName: row.host.discovery?.detected_name ?? row.custom_name,
    }));
}

export function targetsFromInventory(rows: InventoryRow[]): TopologyTarget[] {
  return rows
    .filter((row) => row.current_ip)
    .map((row) => ({
      ip: row.current_ip as string,
      mac: row.mac,
      deviceId: row.device_id,
      hostname: row.hostname,
      detectedName: row.display_name,
    }));
}

export interface DeviceNameLookup {
  byId: Map<number, string>;
  byIp: Map<string, string>;
}

export function nameLookupFromScan(rows: DeviceRow[]): DeviceNameLookup {
  const byId = new Map<number, string>();
  const byIp = new Map<string, string>();
  for (const row of rows) {
    const name =
      row.custom_name ||
      row.host.discovery?.detected_name ||
      row.host.hostname ||
      row.host.ip;
    if (row.device_id != null) byId.set(row.device_id, name);
    byIp.set(row.host.ip, name);
  }
  return { byId, byIp };
}

export function nameLookupFromInventory(rows: InventoryRow[]): DeviceNameLookup {
  const byId = new Map<number, string>();
  const byIp = new Map<string, string>();
  for (const row of rows) {
    if (row.device_id != null) byId.set(row.device_id, row.display_name);
    if (row.current_ip) byIp.set(row.current_ip, row.display_name);
  }
  return { byId, byIp };
}

export function endpointLabel(
  deviceId: number | null | undefined,
  unresolvedId: string | null | undefined,
  names: DeviceNameLookup,
  unknownNodes: UnresolvedNode[],
): string {
  if (deviceId != null && names.byId.has(deviceId)) {
    return names.byId.get(deviceId) as string;
  }
  if (unresolvedId) {
    const node = unknownNodes.find((n) => n.id === unresolvedId);
    if (node?.sysName) return node.sysName;
    if (node?.managementAddress) return node.managementAddress;
    return "Unknown device";
  }
  return "Unknown device";
}

export function confidenceLabel(confidence: TopologyConfidence): string {
  if (confidence === "confirmed") return "Confirmed";
  if (confidence === "strong") return "Strong";
  return "Inferred";
}

export function protocolLabel(protocol: string): string {
  const map: Record<string, string> = {
    lldp: "LLDP",
    cdp: "CDP",
    fdb: "MAC table",
    arp: "ARP",
  };
  return map[protocol] ?? protocol.toUpperCase();
}

export function speedLabel(mbps: number | null | undefined): string | null {
  if (mbps == null || mbps <= 0) return null;
  if (mbps >= 1000 && mbps % 1000 === 0) return `${mbps / 1000} Gbps`;
  return `${mbps} Mbps`;
}

export function vlanLabel(connection: TopologyConnection): string | null {
  if (connection.vlan === "trunk") {
    const tagged = connection.taggedVlans?.length
      ? ` tagged ${connection.taggedVlans.join(", ")}`
      : "";
    const native = connection.nativeVlan != null ? ` native ${connection.nativeVlan}` : "";
    return `Trunk${native}${tagged}`;
  }
  if (connection.vlan) return `VLAN ${connection.vlan}`;
  if (connection.nativeVlan != null) return `VLAN ${connection.nativeVlan}`;
  return null;
}

export function summaryLine(summary: TopologySummary): string {
  const parts = [
    `${summary.confirmed} confirmed`,
    `${summary.strong} strong`,
    `${summary.inferred} inferred`,
  ];
  if (summary.unknownNodes > 0) {
    parts.push(
      `${summary.unknownNodes} unknown ${summary.unknownNodes === 1 ? "neighbour" : "neighbours"}`,
    );
  }
  return parts.join(" · ");
}

/** True when a string looks like it might contain an SNMP secret. */
export function looksLikeSecretLeak(text: string): boolean {
  const lower = text.toLowerCase();
  return (
    /community\s+(?!string\b)\S+/.test(lower) ||
    lower.includes("auth-pass") ||
    lower.includes("priv-pass") ||
    lower.includes("authpassword") ||
    lower.includes("privpassword")
  );
}
