import type { InventoryDiscovery, InventoryRow } from "../../types";
import type { TopologySnapshot } from "../topology";

export const WINDOWS_PASSWORD_FIXTURE = "Win-Fixture-Password!";
export const SNMP_COMMUNITY_FIXTURE = "site-read-fixture";
export const SNMP_AUTH_PASSWORD_FIXTURE = "auth-fixture-secret";
export const SNMP_PRIV_PASSWORD_FIXTURE = "priv-fixture-secret";

const CAPTURED_AT = "2026-09-16T12:00:00.000Z";
const PHYSICAL_SERVER = "uuid||4c4c4544004a37108054b7c04f324d13";

function discovery(patch: Partial<InventoryDiscovery>): InventoryDiscovery {
  return {
    detected_name: null,
    device_type: "unknown",
    type_confidence: "unknown",
    evidence_freshness: "current",
    sources: [],
    manufacturer: null,
    model_name: null,
    services: [],
    last_discovered_at: CAPTURED_AT,
    ...patch,
  };
}

function row(patch: Partial<InventoryRow> & Pick<InventoryRow, "device_id" | "display_name">): InventoryRow {
  const { device_id, display_name, ...rest } = patch;
  return {
    device_id,
    network_scope_id: 1,
    network_name: "Site LAN",
    identity_source: "mac",
    display_name,
    custom_name: null,
    hostname: null,
    current_ip: null,
    previous_ips: [],
    mac: null,
    vendor: null,
    os_guess: null,
    status: "trusted",
    presence: "present",
    first_seen: "2026-09-15T08:00:00.000Z",
    last_seen: CAPTURED_AT,
    last_completed_scan_id: 91,
    last_completed_scan_at: CAPTURED_AT,
    observation_count: 2,
    open_ports: [],
    notes_present: false,
    notes_excerpt: null,
    latest_response_ms: 2,
    latest_icmp_ms: 1.5,
    latest_tcp_ms: 2.5,
    discovery: null,
    user_device_type: null,
    ...rest,
  };
}

const serverDiscovery = discovery({
  detected_name: "APP-01",
  device_type: "server",
  type_confidence: "high",
  sources: ["windows_credentialed"],
  manufacturer: "Dell Inc.",
  model_name: "PowerEdge R750",
  os_family: "Windows",
  os_product: "Windows Server 2022",
  os_edition: "Standard",
  os_version: "2022",
  os_build: "20348",
  os_architecture: "x64",
  windows_product_type: "3",
  hardware_manufacturer: "Dell Inc.",
  hardware_model: "PowerEdge R750",
  hardware_serial: "J7K2M13",
  system_uuid: "4C4C4544-004A-3710-8054-B7C04F324D13",
  domain: "corp.example",
  identity_evidence: ["system UUID: 4C4C4544-004A-3710-8054-B7C04F324D13"],
  identity_sources: ["windows_credentialed"],
  type_evidence: ["Windows ProductType 3 (server)"],
});

/** Real InventoryRow inputs for the actual ArcScan exporter. */
export const V19_INTEGRATION_ROWS: InventoryRow[] = [
  row({
    device_id: 21,
    display_name: "APP-01",
    hostname: "app-01",
    current_ip: "10.0.0.21",
    mac: "02:AA:00:00:00:21",
    vendor: "Dell Inc.",
    os_guess: "Windows",
    open_ports: [80, 443, 5985],
    physical_device_key: PHYSICAL_SERVER,
    physical_interface_count: 2,
    discovery: serverDiscovery,
  }),
  row({
    device_id: 22,
    display_name: "APP-01",
    hostname: "app-01",
    current_ip: "10.0.1.21",
    mac: "02:AA:00:00:01:21",
    vendor: "Dell Inc.",
    os_guess: "Windows",
    open_ports: [80, 443],
    physical_device_key: PHYSICAL_SERVER,
    physical_interface_count: 2,
    discovery: serverDiscovery,
  }),
  row({
    device_id: 30,
    display_name: "APP-01 iDRAC",
    hostname: "idrac-app-01",
    current_ip: "10.0.0.30",
    mac: "02:AA:00:00:00:30",
    vendor: "Dell Inc.",
    open_ports: [443],
    physical_device_key: "serial||idrac-j7k2m13",
    physical_interface_count: 1,
    discovery: discovery({
      detected_name: "APP-01 iDRAC",
      device_type: "management_controller",
      type_confidence: "high",
      sources: ["https"],
      manufacturer: "Dell Inc.",
      model_name: "iDRAC 9",
      type_evidence: ["iDRAC web interface"],
    }),
  }),
  row({
    device_id: 31,
    display_name: "Lobby AP",
    hostname: "lobby-ap",
    current_ip: "10.0.0.31",
    mac: "02:AA:00:00:00:31",
    vendor: "Ubiquiti",
    discovery: discovery({
      detected_name: "Lobby AP",
      device_type: "access_point",
      type_confidence: "high",
      sources: ["mdns"],
      manufacturer: "Ubiquiti",
      model_name: "U7 Pro",
    }),
  }),
  row({
    device_id: 40,
    display_name: "Core Switch",
    hostname: "core-sw",
    current_ip: "10.0.0.40",
    mac: "02:AA:00:00:00:40",
    vendor: "Cisco",
    open_ports: [22, 443, 161],
    discovery: discovery({
      detected_name: "Core Switch",
      device_type: "switch",
      type_confidence: "high",
      sources: ["snmp"],
      manufacturer: "Cisco",
      model_name: "C9300-48P",
    }),
  }),
];

/** Snapshot whose id 22 endpoint proves transport rows are not collapsed. */
export const V19_INTEGRATION_TOPOLOGY: TopologySnapshot = {
  capturedAt: CAPTURED_AT,
  connections: [
    {
      fromDeviceId: 40,
      toDeviceId: 22,
      fromPort: "Gi1/0/20",
      toPort: "Ethernet 2",
      kind: "ethernet",
      protocol: "lldp",
      confidence: "confirmed",
      speedMbps: 1000,
      vlan: "20",
      nativeVlan: 20,
      taggedVlans: [],
      poe: { enabled: false },
      evidence: ["LLDP neighbour declaration"],
    },
    {
      fromDeviceId: 40,
      toDeviceId: 31,
      fromPort: "Gi1/0/31",
      toPort: "eth0",
      kind: "ethernet",
      protocol: "fdb",
      confidence: "strong",
      speedMbps: 2500,
      vlan: "trunk",
      nativeVlan: 10,
      taggedVlans: [10, 20, 30],
      poe: { enabled: true, watts: 13.7 },
      evidence: ["Single-MAC FDB and ARP correlation"],
    },
    {
      fromDeviceId: 40,
      toUnresolvedId: "unknown:chassis:deadbeef0001",
      fromPort: "Gi1/0/48",
      toPort: "Gi0/1",
      kind: "ethernet",
      protocol: "cdp",
      confidence: "confirmed",
      speedMbps: 1000,
      taggedVlans: [],
      evidence: ["CDP neighbour is not in local inventory"],
    },
  ],
  unknownNodes: [
    {
      id: "unknown:chassis:deadbeef0001",
      chassisId: "DE:AD:BE:EF:00:01",
      sysName: "unmanaged-access",
      managementAddress: "10.0.0.250",
      reason: "Neighbour is not present in this scan's inventory.",
      source: "cdp",
    },
  ],
};
