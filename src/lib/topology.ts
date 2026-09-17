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
  fromLogicalId?: string | null;
  toLogicalId?: string | null;
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

export interface LogicalNode {
  id: string;
  kind: string;
  label: string;
  physical: boolean;
}

export interface TopologyEdge {
  gatewayDeviceId?: number | null;
  gatewayIp?: string | null;
  gatewayMac?: string | null;
  internet: LogicalNode;
  viaUnresolvedId?: string | null;
  viaDeviceId?: number | null;
  uplink: TopologyConnection;
  confidence: TopologyConfidence;
  evidence: string[];
}

export interface TopologySnapshot {
  capturedAt: string;
  connections: TopologyConnection[];
  unknownNodes?: UnresolvedNode[];
  logicalNodes?: LogicalNode[];
  edge?: TopologyEdge | null;
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
  gatewayIp?: string | null;
  gatewayMac?: string | null;
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
  if (unresolvedId === INTERNET_NODE_ID || unresolvedId === "logical:internet") {
    return "Internet";
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
    "default-route": "Default route",
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

export const INTERNET_NODE_ID = "logical:internet";

export const TOPOLOGY_HINT =
  "Discover physical network relationships using SNMP and neighbour protocols.";

export const CONFIDENCE_HINT: Record<TopologyConfidence, string> = {
  confirmed: "Direct neighbour evidence from LLDP/CDP.",
  strong: "Matched from switch MAC/FDB evidence.",
  inferred: "Best-effort relationship. Verify before relying on it.",
};

export type PreviewKind =
  | "internet"
  | "firewall"
  | "router"
  | "switch"
  | "access_point"
  | "server"
  | "domain_controller"
  | "nas"
  | "workstation"
  | "computer"
  | "printer"
  | "camera"
  | "unknown";

export type PreviewLayer = "internet" | "wan" | "edge" | "switch" | "infra" | "endpoint";

export interface DeviceTypeLookup {
  byId: Map<number, string>;
}

export function typeLookupFromScan(
  rows: Array<{
    device_id: number | null;
    user_device_type?: string | null;
    host: { discovery?: { device_type?: string | null } | null };
  }>,
  inventory?: Array<{
    device_id: number;
    user_device_type?: string | null;
    discovery?: { device_type?: string | null } | null;
  }>,
): DeviceTypeLookup {
  const byId = new Map<number, string>();
  const remember = (id: number, user: string | null | undefined, detected: string | null | undefined) => {
    const chosen = typeof user === "string" ? user.trim() : "";
    const type = chosen || detected || "unknown";
    byId.set(id, type);
  };
  for (const row of rows) {
    if (row.device_id == null) continue;
    remember(row.device_id, row.user_device_type, row.host.discovery?.device_type);
  }
  for (const row of inventory ?? []) {
    remember(row.device_id, row.user_device_type, row.discovery?.device_type);
  }
  return { byId };
}

export function previewKindFromDeviceType(type: string | null | undefined): PreviewKind {
  switch (type) {
    case "internet":
      return "internet";
    case "firewall":
      return "firewall";
    case "router":
      return "router";
    case "switch":
      return "switch";
    case "access_point":
      return "access_point";
    case "server":
      return "server";
    case "domain_controller":
      return "domain_controller";
    case "nas":
      return "nas";
    case "workstation":
      return "workstation";
    case "computer":
      return "computer";
    case "printer":
      return "printer";
    case "camera":
      return "camera";
    default:
      return "unknown";
  }
}

export function previewLayerForKind(kind: PreviewKind): PreviewLayer {
  switch (kind) {
    case "internet":
      return "internet";
    case "firewall":
    case "router":
      return "edge";
    case "switch":
      return "switch";
    case "access_point":
    case "server":
    case "domain_controller":
    case "nas":
      return "infra";
    default:
      return "endpoint";
  }
}

export function isEndpointLayer(layer: PreviewLayer): boolean {
  return layer === "endpoint";
}

export interface PreviewNode {
  id: string;
  label: string;
  kind: PreviewKind;
  layer: PreviewLayer;
  physical: boolean;
  deviceId?: number;
  unresolvedId?: string;
  logicalId?: string;
  x: number;
  y: number;
}

export interface PreviewEdge {
  id: string;
  from: string;
  to: string;
  connection: TopologyConnection;
}

export const PREVIEW_NODE_WIDTH = 148;
export const PREVIEW_NODE_HEIGHT = 58;
export const PREVIEW_GAP_X = 20;
export const PREVIEW_GAP_Y = 92;
export const PREVIEW_PAD = 28;

const LAYER_ORDER: PreviewLayer[] = ["internet", "wan", "edge", "switch", "infra", "endpoint"];

function endpointId(connection: TopologyConnection, side: "from" | "to"): string | null {
  if (side === "from") {
    if (connection.fromLogicalId) return connection.fromLogicalId;
    if (connection.fromDeviceId != null) return `device:${connection.fromDeviceId}`;
    if (connection.fromUnresolvedId) return connection.fromUnresolvedId;
    return null;
  }
  if (connection.toLogicalId) return connection.toLogicalId;
  if (connection.toDeviceId != null) return `device:${connection.toDeviceId}`;
  if (connection.toUnresolvedId) return connection.toUnresolvedId;
  return null;
}

export function layoutTopology(args: {
  snapshot: TopologySnapshot;
  names: DeviceNameLookup;
  types: DeviceTypeLookup;
  showEndpoints?: boolean;
}): { nodes: PreviewNode[]; edges: PreviewEdge[]; width: number; height: number } {
  const showEndpoints = args.showEndpoints !== false;
  const unknown = args.snapshot.unknownNodes ?? [];
  const logical = args.snapshot.logicalNodes ?? [];
  const nodesById = new Map<string, Omit<PreviewNode, "x" | "y">>();

  const remember = (node: Omit<PreviewNode, "x" | "y">) => {
    if (!nodesById.has(node.id)) nodesById.set(node.id, node);
  };

  for (const node of logical) {
    const kind = node.kind === "internet" ? "internet" : previewKindFromDeviceType(node.kind);
    remember({
      id: node.id,
      label: node.label || "Internet",
      kind,
      layer: kind === "internet" ? "internet" : previewLayerForKind(kind),
      physical: node.physical,
      logicalId: node.id,
    });
  }

  const consider = (connection: TopologyConnection, side: "from" | "to") => {
    const id = endpointId(connection, side);
    if (!id) return;
    if (id.startsWith("logical:")) {
      remember({
        id,
        label: id === INTERNET_NODE_ID ? "Internet" : id,
        kind: id === INTERNET_NODE_ID ? "internet" : "unknown",
        layer: id === INTERNET_NODE_ID ? "internet" : "wan",
        physical: false,
        logicalId: id,
      });
      return;
    }
    if (id.startsWith("device:")) {
      const deviceId = Number(id.slice("device:".length));
      const type = args.types.byId.get(deviceId) ?? "unknown";
      const kind = previewKindFromDeviceType(type);
      remember({
        id,
        label: args.names.byId.get(deviceId) ?? `Device ${deviceId}`,
        kind,
        layer: previewLayerForKind(kind),
        physical: true,
        deviceId,
      });
      return;
    }
    const unresolved = unknown.find((n) => n.id === id);
    const ont =
      looksLikeOntLabel(unresolved?.sysName ?? unresolved?.reason ?? id) ||
      args.snapshot.edge?.viaUnresolvedId === id;
    remember({
      id,
      label: unresolved?.sysName || unresolved?.managementAddress || "Unknown device",
      kind: ont ? "unknown" : "switch",
      layer: ont ? "wan" : unresolved?.source === "lldp" || unresolved?.source === "cdp" ? "switch" : "endpoint",
      physical: true,
      unresolvedId: id,
    });
  };

  const connections = connectionsForPreview(args.snapshot);
  for (const connection of connections) {
    consider(connection, "from");
    consider(connection, "to");
  }

  if (args.snapshot.edge?.internet) {
    remember({
      id: args.snapshot.edge.internet.id,
      label: args.snapshot.edge.internet.label || "Internet",
      kind: "internet",
      layer: "internet",
      physical: false,
      logicalId: args.snapshot.edge.internet.id,
    });
  }

  const viaDeviceId = args.snapshot.edge?.viaDeviceId;
  if (viaDeviceId != null) {
    const id = `device:${viaDeviceId}`;
    const existing = nodesById.get(id);
    if (existing) {
      nodesById.set(id, { ...existing, layer: "wan" });
    }
  }
  const viaUnresolvedId = args.snapshot.edge?.viaUnresolvedId;
  if (viaUnresolvedId) {
    const existing = nodesById.get(viaUnresolvedId);
    if (existing) {
      nodesById.set(viaUnresolvedId, { ...existing, layer: "wan" });
    }
  }

  const filtered = [...nodesById.values()].filter((node) => {
    if (showEndpoints) return true;
    return !isEndpointLayer(node.layer);
  });
  const visible = new Set(filtered.map((node) => node.id));

  const byLayer = new Map<PreviewLayer, typeof filtered>();
  for (const layer of LAYER_ORDER) byLayer.set(layer, []);
  for (const node of filtered) {
    byLayer.get(node.layer)?.push(node);
  }
  for (const list of byLayer.values()) {
    list.sort((a, b) => a.label.localeCompare(b.label));
  }

  const occupiedLayers = LAYER_ORDER.filter((layer) => (byLayer.get(layer) ?? []).length > 0);
  const maxCount = Math.max(1, ...occupiedLayers.map((layer) => (byLayer.get(layer) ?? []).length));
  const width = Math.max(
    360,
    PREVIEW_PAD * 2 + maxCount * PREVIEW_NODE_WIDTH + (maxCount - 1) * PREVIEW_GAP_X,
  );
  const height = Math.max(
    180,
    PREVIEW_PAD * 2 + occupiedLayers.length * PREVIEW_NODE_HEIGHT + (occupiedLayers.length - 1) * PREVIEW_GAP_Y,
  );

  const nodes: PreviewNode[] = [];
  occupiedLayers.forEach((layer, layerIndex) => {
    const list = byLayer.get(layer) ?? [];
    const rowWidth =
      list.length * PREVIEW_NODE_WIDTH + Math.max(0, list.length - 1) * PREVIEW_GAP_X;
    const startX = (width - rowWidth) / 2;
    const y = PREVIEW_PAD + layerIndex * (PREVIEW_NODE_HEIGHT + PREVIEW_GAP_Y);
    list.forEach((node, index) => {
      nodes.push({
        ...node,
        x: startX + index * (PREVIEW_NODE_WIDTH + PREVIEW_GAP_X),
        y,
      });
    });
  });

  const edges: PreviewEdge[] = [];
  connections.forEach((connection, index) => {
    const from = endpointId(connection, "from");
    const to = endpointId(connection, "to");
    if (!from || !to) return;
    if (!visible.has(from) || !visible.has(to)) return;
    edges.push({
      id: `${from}->${to}:${connection.protocol}:${index}`,
      from,
      to,
      connection,
    });
  });

  return { nodes, edges, width, height };
}

function connectionsForPreview(snapshot: TopologySnapshot): TopologyConnection[] {
  const connections = [...snapshot.connections];
  const uplink = snapshot.edge?.uplink;
  if (!uplink) return connections;
  const already = connections.some(
    (connection) =>
      connection.kind === "wan" &&
      (connection.fromLogicalId ?? connection.fromUnresolvedId) ===
        (uplink.fromLogicalId ?? uplink.fromUnresolvedId) &&
      (connection.toDeviceId ?? connection.toUnresolvedId ?? connection.toLogicalId) ===
        (uplink.toDeviceId ?? uplink.toUnresolvedId ?? uplink.toLogicalId),
  );
  if (!already) connections.push(uplink);
  return connections;
}

function looksLikeOntLabel(text: string): boolean {
  const lower = text.toLowerCase();
  return ["ont", "gpon", "modem", "optical network"].some((needle) => lower.includes(needle));
}

export function connectionDetailLines(
  connection: TopologyConnection,
  names: DeviceNameLookup,
  unknownNodes: UnresolvedNode[],
): string[] {
  const from = endpointLabel(
    connection.fromDeviceId,
    connection.fromUnresolvedId ?? connection.fromLogicalId,
    names,
    unknownNodes,
  );
  const to = endpointLabel(
    connection.toDeviceId,
    connection.toUnresolvedId ?? connection.toLogicalId,
    names,
    unknownNodes,
  );
  const lines = [
    `${from}${connection.fromPort ? ` ${connection.fromPort}` : ""} → ${to}${
      connection.toPort ? ` ${connection.toPort}` : ""
    }`,
    `${protocolLabel(connection.protocol)} · ${confidenceLabel(connection.confidence)}`,
  ];
  const speed = speedLabel(connection.speedMbps);
  if (speed) lines.push(speed);
  const vlan = vlanLabel(connection);
  if (vlan) lines.push(vlan);
  if (connection.poe?.enabled) {
    lines.push(connection.poe.watts != null ? `PoE ${connection.poe.watts} W` : "PoE");
  }
  if (connection.evidence[0]) lines.push(connection.evidence[0]);
  return lines;
}
