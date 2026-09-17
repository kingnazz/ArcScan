import { describe, expect, it } from "vitest";
import type { InventoryRow } from "../types";
import type { DeviceRow } from "./live";
import {
  CONFIDENCE_HINT,
  INTERNET_NODE_ID,
  TOPOLOGY_HINT,
  connectionDetailLines,
  credentialInputError,
  emptyCredentialInput,
  endpointLabel,
  layoutTopology,
  looksLikeSecretLeak,
  nameLookupFromInventory,
  protocolLabel,
  speedLabel,
  summaryLine,
  targetsFromInventory,
  targetsFromScanRows,
  typeLookupFromScan,
  vlanLabel,
  type TopologyConnection,
  type TopologySnapshot,
  type TopologySummary,
} from "./topology";

function scanRow(ip: string, deviceId: number | null, mac: string | null, name: string): DeviceRow {
  return {
    host: {
      ip,
      hostname: name,
      mac,
      vendor: null,
      open_ports: [],
      response_ms: 1,
      icmp_ms: 1,
      tcp_ms: null,
      ttl: 64,
      os_guess: null,
      last_seen: "2026-09-16T12:00:00Z",
    },
    device_id: deviceId,
    custom_name: name,
    status: "known",
    first_seen: "2026-09-16T12:00:00Z",
    change: null,
    changed_fields: [],
    pending: false,
  };
}

function previewSnapshot(): TopologySnapshot {
  return {
    capturedAt: "2026-09-16T12:00:00Z",
    connections: [
      {
        fromLogicalId: INTERNET_NODE_ID,
        toDeviceId: 1,
        kind: "wan",
        protocol: "default-route",
        confidence: "strong",
        evidence: ["Default route and gateway MAC both match Home Router."],
      },
      {
        fromDeviceId: 1,
        toDeviceId: 2,
        fromPort: "LAN",
        toPort: "48",
        kind: "ethernet",
        protocol: "lldp",
        confidence: "confirmed",
        evidence: ["LLDP neighbour"],
      },
      {
        fromDeviceId: 2,
        toDeviceId: 4,
        fromPort: "g7",
        kind: "ethernet",
        protocol: "fdb",
        confidence: "strong",
        vlan: "10",
        evidence: ["Exactly one unicast MAC learned on access port g7"],
      },
    ],
    logicalNodes: [{ id: INTERNET_NODE_ID, kind: "internet", label: "Internet", physical: false }],
    edge: {
      gatewayDeviceId: 1,
      gatewayIp: "192.168.1.1",
      gatewayMac: "F4:92:BF:1A:0C:31",
      internet: { id: INTERNET_NODE_ID, kind: "internet", label: "Internet", physical: false },
      uplink: {
        fromLogicalId: INTERNET_NODE_ID,
        toDeviceId: 1,
        kind: "wan",
        protocol: "default-route",
        confidence: "strong",
        evidence: ["Default route and gateway MAC both match Home Router."],
      },
      confidence: "strong",
      evidence: ["Default route and gateway MAC both match Home Router."],
    },
  };
}

describe("topology helpers", () => {
  it("refuses an empty v2c community without suggesting a default", () => {
    const err = credentialInputError(emptyCredentialInput("v2c"));
    expect(err).toMatch(/community string/i);
    expect(err?.toLowerCase()).not.toContain("try public");
    expect(err?.toLowerCase()).not.toContain("try private");
  });

  it("refuses SNMPv3 with no authentication", () => {
    const err = credentialInputError({
      ...emptyCredentialInput("v3"),
      username: "monitor",
      authProtocol: "",
      authPassword: "",
    });
    expect(err).toMatch(/noAuthNoPriv/i);
  });

  it("allows SNMPv3 authNoPriv when privacy is omitted", () => {
    expect(
      credentialInputError({
        ...emptyCredentialInput("v3"),
        username: "monitor",
        authProtocol: "sha256",
        authPassword: "auth-secret",
        privProtocol: "",
        privPassword: "",
      }),
    ).toBeNull();
  });

  it("requires a privacy password only when a privacy protocol is chosen", () => {
    expect(
      credentialInputError({
        ...emptyCredentialInput("v3"),
        username: "monitor",
        authProtocol: "sha256",
        authPassword: "auth-secret",
        privProtocol: "aes128",
        privPassword: "",
      }),
    ).toMatch(/privacy password/i);
    expect(
      credentialInputError({
        ...emptyCredentialInput("v3"),
        username: "monitor",
        authProtocol: "sha256",
        authPassword: "auth-secret",
        privProtocol: "aes128",
        privPassword: "priv-secret",
      }),
    ).toBeNull();
  });

  it("waits for persisted local inventory ids instead of inventing topology ids", () => {
    const targets = targetsFromScanRows([
      scanRow("192.168.1.2", 2, "00:1A:2B:00:00:02", "core-sw"),
      scanRow("192.168.1.50", null, "AA:BB:CC:00:00:50", "workstation"),
    ]);
    expect(targets).toEqual([
      {
        ip: "192.168.1.2",
        mac: "00:1A:2B:00:00:02",
        deviceId: 2,
        hostname: "core-sw",
        detectedName: "core-sw",
      },
    ]);
  });

  it("builds inventory targets only for rows that currently have an address", () => {
    const rows = [
      { device_id: 1, current_ip: "192.168.1.1", mac: "AA:AA:AA:00:00:01", hostname: "gw", display_name: "Router" },
      { device_id: 2, current_ip: null, mac: "BB:BB:BB:00:00:02", hostname: "old", display_name: "Missing PC" },
    ] as InventoryRow[];
    const targets = targetsFromInventory(rows);
    expect(targets).toHaveLength(1);
    expect(targets[0].deviceId).toBe(1);
    expect(targets[0].detectedName).toBe("Router");
  });

  it("labels unresolved neighbours without inventing a vendor", () => {
    const names = nameLookupFromInventory([
      { device_id: 2, current_ip: "192.168.1.2", display_name: "Core Switch" } as InventoryRow,
    ]);
    expect(endpointLabel(2, null, names, [])).toBe("Core Switch");
    expect(
      endpointLabel(null, "unknown:chassis:deadbeef0001", names, [
        {
          id: "unknown:chassis:deadbeef0001",
          sysName: "mystery-sw",
          reason: "LLDP neighbour is not present in this scan's inventory.",
          source: "lldp",
        },
      ]),
    ).toBe("mystery-sw");
    expect(endpointLabel(null, "unknown:unidentified", names, [])).toBe("Unknown device");
    expect(endpointLabel(null, INTERNET_NODE_ID, names, [])).toBe("Internet");
  });

  it("formats speed, vlan and summary the way the panel shows them", () => {
    const trunk: TopologyConnection = {
      kind: "ethernet",
      protocol: "lldp",
      confidence: "confirmed",
      vlan: "trunk",
      nativeVlan: 10,
      taggedVlans: [10, 20, 30],
      evidence: [],
    };
    expect(vlanLabel(trunk)).toBe("Trunk native 10 tagged 10, 20, 30");
    expect(speedLabel(1000)).toBe("1 Gbps");
    expect(speedLabel(100)).toBe("100 Mbps");
    expect(protocolLabel("default-route")).toBe("Default route");
    const summary: TopologySummary = {
      devicesQueried: 4,
      devicesResponded: 1,
      devicesFailed: 3,
      confirmed: 2,
      strong: 1,
      inferred: 0,
      unknownNodes: 1,
      durationMs: 800,
      cancelled: false,
      timedOut: false,
      failures: [],
    };
    expect(summaryLine(summary)).toBe("2 confirmed · 1 strong · 0 inferred · 1 unknown neighbour");
  });

  it("treats community-plus-value as a secret leak, but not the word itself", () => {
    expect(looksLikeSecretLeak("Enter an SNMP community string.")).toBe(false);
    expect(looksLikeSecretLeak("unknown community site-read on agent")).toBe(true);
  });

  it("keeps tooltip copy short and specific", () => {
    expect(TOPOLOGY_HINT).toMatch(/SNMP/);
    expect(CONFIDENCE_HINT.confirmed).toMatch(/LLDP\/CDP/);
    expect(CONFIDENCE_HINT.strong).toMatch(/FDB/);
    expect(CONFIDENCE_HINT.inferred).toMatch(/Verify/);
  });
});

describe("topology preview layout", () => {
  const names = {
    byId: new Map<number, string>([
      [1, "Home Router"],
      [2, "Core Switch"],
      [4, "Home NAS"],
    ]),
    byIp: new Map<string, string>(),
  };
  const types = {
    byId: new Map<number, string>([
      [1, "router"],
      [2, "switch"],
      [4, "nas"],
    ]),
  };

  it("stacks Internet above the gateway, then switches, then devices", () => {
    const layout = layoutTopology({ snapshot: previewSnapshot(), names, types });
    const internet = layout.nodes.find((node) => node.id === INTERNET_NODE_ID);
    const router = layout.nodes.find((node) => node.deviceId === 1);
    const sw = layout.nodes.find((node) => node.deviceId === 2);
    const nas = layout.nodes.find((node) => node.deviceId === 4);
    expect(internet?.physical).toBe(false);
    expect(internet?.kind).toBe("internet");
    expect(router?.layer).toBe("edge");
    expect(sw?.layer).toBe("switch");
    expect(nas?.layer).toBe("infra");
    expect(internet!.y).toBeLessThan(router!.y);
    expect(router!.y).toBeLessThan(sw!.y);
    expect(sw!.y).toBeLessThan(nas!.y);
    expect(layout.edges.some((edge) => edge.connection.kind === "wan")).toBe(true);
  });

  it("hides the endpoint layer without dropping infrastructure", () => {
    const snapshot: TopologySnapshot = {
      ...previewSnapshot(),
      connections: [
        ...previewSnapshot().connections,
        {
          fromDeviceId: 2,
          toDeviceId: 9,
          fromPort: "Port 9",
          kind: "ethernet",
          protocol: "fdb",
          confidence: "strong",
          evidence: ["Exactly one unicast MAC on access port Port 9"],
        },
      ],
    };
    const withPc = {
      byId: new Map([...names.byId.entries(), [9, "Study Desktop"]]),
      byIp: names.byIp,
    };
    const withPcTypes = { byId: new Map([...types.byId.entries(), [9, "workstation"]]) };
    const shown = layoutTopology({ snapshot, names: withPc, types: withPcTypes, showEndpoints: true });
    const hidden = layoutTopology({ snapshot, names: withPc, types: withPcTypes, showEndpoints: false });
    expect(shown.nodes.some((node) => node.deviceId === 9)).toBe(true);
    expect(hidden.nodes.some((node) => node.deviceId === 9)).toBe(false);
    expect(hidden.nodes.some((node) => node.deviceId === 2)).toBe(true);
    expect(hidden.nodes.some((node) => node.id === INTERNET_NODE_ID)).toBe(true);
  });

  it("does not invent an endpoint port when FDB only knows the switch side", () => {
    const namesOnly = { byId: new Map([[2, "Core Switch"], [4, "Home NAS"]]), byIp: new Map<string, string>() };
    const lines = connectionDetailLines(
      {
        fromDeviceId: 2,
        toDeviceId: 4,
        fromPort: "g7",
        toPort: null,
        kind: "ethernet",
        protocol: "fdb",
        confidence: "strong",
        evidence: ["Exactly one unicast MAC learned on access port g7"],
      },
      namesOnly,
      [],
    );
    expect(lines[0]).toBe("Core Switch g7 → Home NAS");
    expect(lines[0]).not.toMatch(/Home NAS \S/);
  });

  it("never treats the Internet node as a numbered inventory device", () => {
    const layout = layoutTopology({ snapshot: previewSnapshot(), names, types });
    const internet = layout.nodes.find((node) => node.kind === "internet");
    expect(internet?.deviceId).toBeUndefined();
    expect(internet?.physical).toBe(false);
    expect(internet?.id).toBe(INTERNET_NODE_ID);
  });

  it("uses inventory classification when scan rows omitted discovery", () => {
    const types = typeLookupFromScan(
      [{ device_id: 1, host: { discovery: null } }],
      [
        {
          device_id: 1,
          user_device_type: null,
          discovery: { device_type: "router" },
        },
      ],
    );
    expect(types.byId.get(1)).toBe("router");
    const overridden = typeLookupFromScan(
      [{ device_id: 1, host: { discovery: { device_type: "nas" } } }],
      [{ device_id: 1, user_device_type: "firewall", discovery: { device_type: "nas" } }],
    );
    expect(overridden.byId.get(1)).toBe("firewall");
  });

  it("places a known inventory ONT between Internet and the gateway", () => {
    const snapshot: TopologySnapshot = {
      capturedAt: "2026-09-16T12:00:00Z",
      connections: [
        {
          fromLogicalId: INTERNET_NODE_ID,
          toDeviceId: 9,
          kind: "wan",
          protocol: "default-route",
          confidence: "strong",
          evidence: ["LLDP neighbour ONT-01 sits between the default gateway and the WAN."],
        },
        {
          fromDeviceId: 1,
          toDeviceId: 9,
          fromPort: "X1",
          toPort: "gpon0",
          kind: "ethernet",
          protocol: "lldp",
          confidence: "confirmed",
          evidence: ["LLDP neighbour on Home Router reports ONT-01"],
        },
      ],
      logicalNodes: [{ id: INTERNET_NODE_ID, kind: "internet", label: "Internet", physical: false }],
      edge: {
        gatewayDeviceId: 1,
        viaDeviceId: 9,
        internet: { id: INTERNET_NODE_ID, kind: "internet", label: "Internet", physical: false },
        uplink: {
          fromLogicalId: INTERNET_NODE_ID,
          toDeviceId: 9,
          kind: "wan",
          protocol: "default-route",
          confidence: "strong",
          evidence: ["LLDP neighbour ONT-01 sits between the default gateway and the WAN."],
        },
        confidence: "strong",
        evidence: [],
      },
    };
    const layout = layoutTopology({
      snapshot,
      names: {
        byId: new Map([
          [1, "Home Router"],
          [9, "ONT-01"],
        ]),
        byIp: new Map(),
      },
      types: {
        byId: new Map([
          [1, "firewall"],
          [9, "unknown"],
        ]),
      },
    });
    const internet = layout.nodes.find((node) => node.id === INTERNET_NODE_ID);
    const ont = layout.nodes.find((node) => node.deviceId === 9);
    const gateway = layout.nodes.find((node) => node.deviceId === 1);
    expect(ont?.layer).toBe("wan");
    expect(gateway?.layer).toBe("edge");
    expect(internet!.y).toBeLessThan(ont!.y);
    expect(ont!.y).toBeLessThan(gateway!.y);
    expect(layout.edges.some((edge) => edge.connection.toDeviceId === 9 && edge.connection.kind === "wan")).toBe(
      true,
    );
  });
});
