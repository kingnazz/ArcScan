import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { TopologyPanel } from "./TopologyPanel";
import { TopologyPreview } from "./TopologyPreview";
import { EMPTY_CREDENTIAL_STATUS, type TopologyDiagnostics, type TopologyResult } from "../lib/topology";

afterEach(() => cleanup());

const names = {
  byId: new Map<number, string>([
    [2, "NETGEAR-SW1"],
    [4, "BC-NAS1"],
  ]),
  byIp: new Map<string, string>(),
};

function diagnostics(): TopologyDiagnostics {
  return {
    kind: "arcscan-topology-diagnostics",
    devices: [
      {
        targetIp: "192.168.60.2",
        displayName: "NETGEAR-SW1",
        inventoryDeviceId: 2,
        snmpStatus: "responded",
        sysName: "NETGEAR-SW1",
        mibCoverage: [
          { mib: "IF-MIB", state: "available", rows: 8 },
          { mib: "LLDP-MIB", state: "available", rows: 4, detail: "Local port table answered. Remote neighbour table returned no rows." },
          { mib: "BRIDGE-MIB", state: "available", rows: 137 },
        ],
        interfaces: { count: 8, up: 6, withName: 8, withSpeed: 8 },
        lldp: { state: "noRows", neighbourCount: 0, resolved: 0, unresolved: 0, neighbours: [] },
        cdp: { state: "noRows", neighbourCount: 0, resolved: 0, unresolved: 0, neighbours: [] },
        fdb: {
          state: "available",
          totalRows: 400,
          unicastMacs: 19,
          matchedInventory: 2,
          unresolvedMacs: 17,
          singleMacPorts: 2,
          multiMacPorts: 3,
          strongLinks: 1,
          uplinkSuppressions: 3,
          portsOmitted: 28,
          ports: [
            {
              portLabel: "Port 24",
              rawPort: 24,
              relevantMacs: 14,
              matchedInventory: 0,
              outcome: "suppressed-uplink",
              summary:
                "Port 24 learned 14 relevant unicast MACs. The relationship was suppressed because this resembles an uplink or trunk rather than a directly attached endpoint.",
            },
          ],
        },
        arp: { state: "available", entries: 12, fdbCorroborations: 1 },
        vlan: { state: "available", pvidPorts: 8, accessPorts: 6, trunkPorts: 1 },
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
        relationshipCount: 40,
        relationshipsOmitted: 39,
        unresolvedPeers: 0,
        suppressions: [
          {
            targetIp: "192.168.60.2",
            reason: "multi-mac-uplink",
            summary:
              "Port 24 learned 14 relevant unicast MACs. The relationship was suppressed because this resembles an uplink or trunk rather than a directly attached endpoint.",
            portLabel: "Port 24",
            macCount: 14,
          },
        ],
        suppressionsOmitted: 2,
        portMappings: [
          {
            role: "fdb",
            rawPort: 12,
            resolvedIfIndex: 12,
            displayLabel: "Port 12",
            resolutionSource: "ifIndex 12 matched IF-MIB directly",
            fellBackToNumeric: false,
          },
        ],
        notes: [],
        hints: ["LLDP may be disabled on this switch. FDB topology is still available."],
      },
      {
        targetIp: "192.168.60.5",
        snmpStatus: "timeout",
        failureReason: "The device did not answer SNMP in time.",
        mibCoverage: [{ mib: "LLDP-MIB", state: "notQueried", rows: 0 }],
        interfaces: { count: 0, up: 0, withName: 0, withSpeed: 0 },
        lldp: { state: "notQueried", neighbourCount: 0, resolved: 0, unresolved: 0, neighbours: [] },
        cdp: { state: "notQueried", neighbourCount: 0, resolved: 0, unresolved: 0, neighbours: [] },
        fdb: {
          state: "notQueried",
          totalRows: 0,
          unicastMacs: 0,
          matchedInventory: 0,
          unresolvedMacs: 0,
          singleMacPorts: 0,
          multiMacPorts: 0,
          strongLinks: 0,
          uplinkSuppressions: 0,
          portsOmitted: 0,
          ports: [],
        },
        arp: { state: "notQueried", entries: 0, fdbCorroborations: 0 },
        vlan: { state: "notQueried", pvidPorts: 0, accessPorts: 0, trunkPorts: 0 },
        poe: {
          detectionState: "notQueried",
          wattageState: "notQueried",
          enabledPorts: 0,
          portsWithWatts: 0,
          enabledWithoutWatts: 0,
        },
        relationships: [],
        relationshipCount: 0,
        relationshipsOmitted: 0,
        unresolvedPeers: 0,
        suppressions: [],
        suppressionsOmitted: 0,
        portMappings: [],
        notes: [],
        hints: [],
        zeroLinkExplanation: "SNMP timed out. No topology tables were read from this device.",
      },
    ],
    runSummary: {
      devicesQueried: 2,
      devicesResponding: 1,
      devicesFailed: 1,
      lldpCdpNeighbours: 0,
      fdbRelationships: 1,
      confirmedLinks: 0,
      strongLinks: 1,
      inferredLinks: 0,
      unresolvedNeighbours: 0,
      suppressedCandidates: 3,
      partialSnmpDevices: 0,
    },
    suppressions: [],
    correlationNotes: [],
  };
}

const result: TopologyResult = {
  snapshot: {
    capturedAt: "2026-09-21T12:00:00Z",
    connections: [
      {
        fromDeviceId: 2,
        toDeviceId: 4,
        fromPort: "Port 12",
        kind: "ethernet",
        protocol: "fdb",
        confidence: "strong",
        evidence: [
          "Exactly one unicast MAC learned on access port Port 12",
          "ARP maps AA:BB:CC:00:00:20 to 192.168.60.20",
        ],
      },
    ],
    unknownNodes: [],
  },
  summary: {
    devicesQueried: 2,
    devicesResponded: 1,
    devicesFailed: 1,
    confirmed: 0,
    strong: 1,
    inferred: 0,
    unknownNodes: 0,
    durationMs: 400,
    cancelled: false,
    timedOut: false,
    failures: [],
  },
  diagnostics: diagnostics(),
};

describe("Topology diagnostics panel", () => {
  it("shows run facts, expands one device, and does not render the whole FDB", () => {
    const exportReplay = vi.fn().mockResolvedValue(true);
    render(
      <TopologyPanel
        credentialStatus={{ ...EMPTY_CREDENTIAL_STATUS, configured: true, version: "v2c" }}
        result={result}
        names={names}
        types={{ byId: new Map([[2, "switch"], [4, "nas"]]) }}
        targetCount={2}
        busy={false}
        error={null}
        onSaveCredentials={async () => undefined}
        onClearCredentials={async () => undefined}
        onDiscover={async () => undefined}
        onExportReplay={exportReplay}
        onCancel={() => undefined}
        onBack={() => undefined}
      />,
    );
    expect(screen.getByText("Topology diagnostics")).toBeTruthy();
    expect(screen.getByText("Suppressed candidates")).toBeTruthy();
    expect(screen.getAllByText(/network inventory information/).length).toBeGreaterThan(0);
    expect(screen.getByRole("button", { name: /192\.168\.60\.2/ }).textContent).toMatch(/LLDP No neighbours/);
    expect(screen.getByRole("button", { name: /192\.168\.60\.2/ }).textContent).toMatch(/Links 40/);
    expect(screen.getByRole("button", { name: /192\.168\.60\.2/ }).textContent).not.toMatch(/LLDP Available/);
    expect(screen.getByText("192.168.60.5")).toBeTruthy();
    expect(screen.getByText(/SNMP timeout/)).toBeTruthy();
    expect(screen.queryByText("site-read-secret")).toBeNull();
    expect(screen.getByText(/machine-readable parsed evidence/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Export replay fixture" }));
    expect(exportReplay).toHaveBeenCalledOnce();

    fireEvent.click(screen.getByRole("button", { name: /192\.168\.60\.2/ }));
    expect(screen.getAllByText(/14 relevant unicast MACs/).length).toBeGreaterThan(0);
    expect(screen.getByText(/additional ports are included in the counts only/)).toBeTruthy();
    const page = document.body.textContent ?? "";
    expect(page).toContain("400");
    expect(page.match(/learned \d+ relevant/g)?.length ?? 0).toBeLessThan(5);
  });
});

describe("Topology preview explainability", () => {
  it("keeps the card compact and reveals every evidence line on demand", () => {
    render(
      <TopologyPreview
        snapshot={result.snapshot}
        names={names}
        types={{ byId: new Map([[2, "switch"], [4, "nas"]]) }}
        diagnostics={result.diagnostics}
      />,
    );
    const edge = screen.getByRole("button", { name: /NETGEAR-SW1 Port 12/i });
    fireEvent.keyDown(edge, { key: "Enter" });
    expect(screen.getByText("Why this connection?")).toBeTruthy();
    const details = screen.getByText("Why this connection?").closest("details");
    expect(details?.open).toBe(false);
    fireEvent.click(screen.getByText("Why this connection?"));
    expect(details?.open).toBe(true);
    expect(screen.getByText(/ARP independently associates/)).toBeTruthy();
    expect(screen.getAllByText(/ARP maps AA:BB:CC:00:00:20/).length).toBeGreaterThan(0);
  });
});
