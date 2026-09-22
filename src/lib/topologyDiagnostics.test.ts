import { describe, expect, it } from "vitest";
import {
  formatDiagnosticsExport,
  whyForConnection,
  type DeviceTopologyDiagnostics,
  type TopologyConnection,
  type TopologyDiagnostics,
} from "./topology";

function device(partial: Partial<DeviceTopologyDiagnostics> = {}): DeviceTopologyDiagnostics {
  return {
    targetIp: "192.168.60.2",
    displayName: "NETGEAR-SW1",
    snmpStatus: "responded",
    mibCoverage: [{ mib: "LLDP-MIB", state: "noRows", rows: 0 }],
    interfaces: { count: 4, up: 4, withName: 4, withSpeed: 4 },
    lldp: { state: "noRows", neighbourCount: 0, resolved: 0, unresolved: 0, neighbours: [] },
    cdp: { state: "noRows", neighbourCount: 0, resolved: 0, unresolved: 0, neighbours: [] },
    fdb: {
      state: "available",
      totalRows: 4,
      unicastMacs: 1,
      matchedInventory: 1,
      unresolvedMacs: 0,
      singleMacPorts: 1,
      multiMacPorts: 0,
      strongLinks: 1,
      uplinkSuppressions: 0,
      portsOmitted: 0,
      ports: [],
    },
    arp: { state: "available", entries: 1, fdbCorroborations: 1 },
    vlan: { state: "available", pvidPorts: 1, accessPorts: 1, trunkPorts: 0 },
    poe: {
      detectionState: "available",
      wattageState: "noRows",
      enabledPorts: 1,
      portsWithWatts: 0,
      enabledWithoutWatts: 1,
    },
    relationships: [
      {
        protocol: "fdb",
        confidence: "strong",
        fromDeviceId: 2,
        toDeviceId: 4,
        fromPort: "Port 12",
        resolution: "fdb-mac",
        why: "Switch FDB learned exactly one relevant inventory MAC on port Port 12. MAC belongs to BC-NAS1. ARP independently associates that MAC with 192.168.60.20.",
      },
    ],
    relationshipCount: 1,
    relationshipsOmitted: 0,
    unresolvedPeers: 0,
    suppressions: [],
    suppressionsOmitted: 0,
    portMappings: [],
    notes: [],
    hints: ["LLDP may be disabled on this switch. FDB topology is still available."],
    ...partial,
  };
}

function diagnostics(devices: DeviceTopologyDiagnostics[]): TopologyDiagnostics {
  return {
    kind: "arcscan-topology-diagnostics",
    devices,
    runSummary: {
      devicesQueried: devices.length,
      devicesResponding: devices.filter((item) => item.snmpStatus === "responded").length,
      devicesFailed: devices.filter((item) => item.snmpStatus !== "responded").length,
      lldpCdpNeighbours: 0,
      fdbRelationships: 1,
      confirmedLinks: 0,
      strongLinks: 1,
      inferredLinks: 0,
      unresolvedNeighbours: 0,
      suppressedCandidates: 0,
      partialSnmpDevices: 0,
    },
    suppressions: [],
    correlationNotes: [],
  };
}

describe("topology diagnostics export", () => {
  it("labels inventory data and strips credential material", () => {
    const payload = diagnostics([
      device({
        notes: [
          "community site-read-secret username monitor-user auth_password hunter2 priv_password priv-pass-xyz",
        ],
      }),
    ]);
    const json = formatDiagnosticsExport(payload);
    expect(json).toContain("network inventory information");
    expect(json).not.toContain("site-read-secret");
    expect(json).not.toContain("monitor-user");
    expect(json).not.toContain("hunter2");
    expect(json).not.toContain("priv-pass-xyz");
  });

  it("scrubs equals syntax, camelCase keys, repeats, and nested values", () => {
    const payload = diagnostics([
      device({
        notes: ["community=site-read community=site-read-two username=monitor-user"],
        hints: ["authPass=auth-secret privPass=priv-secret authPass=auth-again"],
      }),
    ]);
    const nested = {
      ...payload,
      correlationNotes: [
        {
          targetIp: "192.168.60.2",
          summary: "community=nested-secret username=nested-user",
          authPass: "camel-auth",
          privPass: "camel-priv",
        },
      ],
    } as unknown as TopologyDiagnostics;
    const json = formatDiagnosticsExport(nested);
    for (const secret of [
      "site-read",
      "site-read-two",
      "monitor-user",
      "auth-secret",
      "priv-secret",
      "auth-again",
      "nested-secret",
      "nested-user",
      "camel-auth",
      "camel-priv",
    ]) {
      expect(json).not.toContain(secret);
    }
    expect(json).not.toMatch(/"authPass"/);
    expect(json).not.toMatch(/"privPass"/);
    expect(json.match(/\[redacted\]/g)?.length ?? 0).toBeGreaterThanOrEqual(6);
  });

  it("labels an empty remote LLDP table as no neighbours even when the MIB aggregate answered", () => {
    const row = device({
      mibCoverage: [{ mib: "LLDP-MIB", state: "available", rows: 4 }],
      lldp: { state: "noRows", neighbourCount: 0, resolved: 0, unresolved: 0, neighbours: [] },
      relationshipCount: 40,
      relationshipsOmitted: 16,
    });
    expect(row.mibCoverage[0]?.state).toBe("available");
    expect(row.lldp.state).toBe("noRows");
  });

  it("finds the structured reason for a connection", () => {
    const connection: TopologyConnection = {
      fromDeviceId: 2,
      toDeviceId: 4,
      fromPort: "Port 12",
      kind: "ethernet",
      protocol: "fdb",
      confidence: "strong",
      evidence: ["Exactly one unicast MAC learned on access port Port 12"],
    };
    const why = whyForConnection(connection, diagnostics([device()]));
    expect(why).toMatch(/ARP independently/);
  });
});
