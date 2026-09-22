// ArcScan <-> ArcAtlas VLAN contract.
//
// ArcAtlas accepts Ethernet VLAN IDs in 1-4094 and rejects a whole topology
// handoff over a single ID outside it. ArcScan therefore normalizes VLAN facts
// before serialization: in range is preserved, anything else is unknown, and
// nothing is ever clamped. These tests cover the last gate, where the schema v2
// envelope is assembled.

import { describe, expect, it } from "vitest";
import { buildHandoffEnvelope, type ArcAtlasHandoffV2 } from "./arcatlas";
import {
  isValidVlanId,
  normalizeConnectionVlans,
  normalizeVlanId,
  normalizeVlanIds,
  normalizeVlanLabel,
  type TopologyConnection,
  type TopologySnapshot,
} from "./topology";
import { V19_INTEGRATION_ROWS } from "./fixtures/v1.9Integration";
import {
  VLAN_CONTRACT_TOPOLOGY,
  VLAN_OUT_OF_RANGE_TOPOLOGY,
} from "./fixtures/vlanContract";

function envelope(topology: TopologySnapshot): ArcAtlasHandoffV2 {
  const built = buildHandoffEnvelope({
    rows: V19_INTEGRATION_ROWS,
    notes: new Map(),
    networkName: "Site LAN",
    handoffId: "00000000-0000-4000-8000-0000000004094",
    sourceVersion: "1.9.0",
    generatedAt: "2026-09-22T12:00:05.000Z",
    topology,
  });
  if (built.schemaVersion !== 2) throw new Error("expected schema v2");
  return built;
}

/** Every VLAN id anywhere in the serialized envelope. */
function vlanIdsIn(handoff: ArcAtlasHandoffV2): number[] {
  const ids: number[] = [];
  const walk = (connections: TopologyConnection[]) => {
    for (const connection of connections) {
      if (connection.nativeVlan != null) ids.push(connection.nativeVlan);
      for (const tagged of connection.taggedVlans ?? []) ids.push(tagged);
      if (connection.vlan != null && connection.vlan !== "trunk") {
        ids.push(Number(connection.vlan));
      }
    }
  };
  walk(handoff.topology.connections);
  walk(handoff.unresolvedTopology?.connections ?? []);
  return ids;
}

describe("VLAN id normalization", () => {
  it("treats 0 as unknown rather than VLAN 1", () => {
    expect(normalizeVlanId(0)).toBeUndefined();
    expect(isValidVlanId(0)).toBe(false);
  });

  it("preserves the contract edges 1 and 4094", () => {
    expect(normalizeVlanId(1)).toBe(1);
    expect(normalizeVlanId(4094)).toBe(4094);
  });

  it("treats 4095 as unknown and never clamps it into range", () => {
    expect(normalizeVlanId(4095)).toBeUndefined();
    expect(normalizeVlanId(4096)).toBeUndefined();
  });

  it("treats a very large value as unknown, never truncating it into range", () => {
    // Each of these would land inside 1-4094 under a 16-bit truncation.
    expect(65546 & 0xffff).toBe(10);
    expect(normalizeVlanId(65546)).toBeUndefined();
    expect(4294971390 & 0xffff).toBe(4094);
    expect(normalizeVlanId(4294971390)).toBeUndefined();
    expect(normalizeVlanId(Number.MAX_SAFE_INTEGER)).toBeUndefined();
  });

  it("rejects values that are not whole VLAN ids", () => {
    expect(normalizeVlanId(-1)).toBeUndefined();
    expect(normalizeVlanId(10.5)).toBeUndefined();
    expect(normalizeVlanId(Number.NaN)).toBeUndefined();
    expect(normalizeVlanId(null)).toBeUndefined();
    expect(normalizeVlanId(undefined)).toBeUndefined();
  });

  it("filters a tagged list one entry at a time", () => {
    expect(normalizeVlanIds([0, 1, 100, 4094, 4095])).toEqual([1, 100, 4094]);
    expect(normalizeVlanIds([0, 4095])).toEqual([]);
    expect(normalizeVlanIds([65546, 20])).toEqual([20]);
    expect(normalizeVlanIds(undefined)).toEqual([]);
  });

  it("keeps trunk and in-range labels only", () => {
    expect(normalizeVlanLabel("trunk")).toBe("trunk");
    expect(normalizeVlanLabel("1")).toBe("1");
    expect(normalizeVlanLabel("4094")).toBe("4094");
    expect(normalizeVlanLabel("0")).toBeUndefined();
    expect(normalizeVlanLabel("4095")).toBeUndefined();
    expect(normalizeVlanLabel("65546")).toBeUndefined();
    expect(normalizeVlanLabel("-1")).toBeUndefined();
    expect(normalizeVlanLabel("")).toBeUndefined();
    expect(normalizeVlanLabel(null)).toBeUndefined();
  });

  it("drops one VLAN fact without touching the rest of the link", () => {
    const connection: TopologyConnection = {
      fromDeviceId: 40,
      toDeviceId: 31,
      fromPort: "Gi1/0/36",
      toPort: "eth0",
      kind: "ethernet",
      protocol: "cdp",
      confidence: "confirmed",
      speedMbps: 1000,
      vlan: "0",
      nativeVlan: 0,
      taggedVlans: [0, 30, 4095],
      poe: { enabled: true, watts: 8.2 },
      evidence: ["CDP neighbour on Gi1/0/36"],
    };
    const normalized = normalizeConnectionVlans(connection);
    expect(normalized).not.toHaveProperty("nativeVlan");
    expect(normalized).not.toHaveProperty("vlan");
    expect(normalized.taggedVlans).toEqual([30]);
    expect(normalized.fromPort).toBe("Gi1/0/36");
    expect(normalized.toPort).toBe("eth0");
    expect(normalized.speedMbps).toBe(1000);
    expect(normalized.poe).toEqual({ enabled: true, watts: 8.2 });
    expect(normalized.evidence).toEqual(["CDP neighbour on Gi1/0/36"]);
    // The caller's snapshot is untouched.
    expect(connection.nativeVlan).toBe(0);
  });
});

describe("schema v2 handoff VLAN contract", () => {
  it("carries no VLAN id outside 1-4094", () => {
    const handoff = envelope(VLAN_OUT_OF_RANGE_TOPOLOGY);
    const ids = vlanIdsIn(handoff);
    expect(ids.length).toBeGreaterThan(0);
    for (const id of ids) {
      expect(isValidVlanId(id)).toBe(true);
    }
    expect(JSON.parse(JSON.stringify(handoff))).toEqual(handoff);
  });

  it("emits the link when a CDP neighbour reports native VLAN 0", () => {
    // The real compatibility failure: one nativeVlan of 0 used to make
    // ArcAtlas reject the entire topology handoff.
    const handoff = envelope(VLAN_OUT_OF_RANGE_TOPOLOGY);
    const link = handoff.topology.connections.find((c) => c.protocol === "cdp");
    expect(link).toBeDefined();
    expect(link?.fromDeviceId).toBe(40);
    expect(link?.toDeviceId).toBe(31);
    expect(link?.fromPort).toBe("Gi1/0/36");
    expect(link?.toPort).toBe("eth0");
    expect(link?.confidence).toBe("confirmed");
    expect(link).not.toHaveProperty("nativeVlan");
    expect(link).not.toHaveProperty("vlan");
    // Absent on the wire too, not null and not 0.
    expect(JSON.stringify(handoff)).not.toContain('"nativeVlan":0');
  });

  it("keeps otherwise valid topology when other links carry invalid VLANs", () => {
    const handoff = envelope(VLAN_OUT_OF_RANGE_TOPOLOGY);
    expect(handoff.topology.connections).toHaveLength(
      VLAN_OUT_OF_RANGE_TOPOLOGY.connections.length,
    );
    const ports = handoff.topology.connections.map((c) => c.fromPort);
    expect(ports).toEqual(["Gi1/0/36", "Gi1/0/20", "Gi1/0/21", "Gi1/0/30"]);

    // The link whose VLANs were all valid is byte-for-byte what it was.
    const untouched = handoff.topology.connections.find((c) => c.fromPort === "Gi1/0/30");
    expect(untouched?.vlan).toBe("10");
    expect(untouched?.nativeVlan).toBe(10);
    expect(untouched?.taggedVlans).toEqual([10, 20]);

    // Mixed lists lose only their invalid entries.
    const mixed = handoff.topology.connections.find((c) => c.fromPort === "Gi1/0/20");
    expect(mixed?.taggedVlans).toEqual([1, 100, 4094]);
    const wide = handoff.topology.connections.find((c) => c.fromPort === "Gi1/0/21");
    expect(wide?.taggedVlans).toEqual([20]);
    expect(wide).not.toHaveProperty("nativeVlan");
  });

  it("hands over the contract-edge fixture unchanged", () => {
    const handoff = envelope(VLAN_CONTRACT_TOPOLOGY);
    expect(handoff.topology.connections).toHaveLength(1);
    const link = handoff.topology.connections[0];
    expect(link).not.toHaveProperty("nativeVlan");
    expect(link.vlan).toBe("trunk");
    expect(link.taggedVlans).toEqual([1, 100, 4094]);
    for (const id of vlanIdsIn(handoff)) {
      expect(isValidVlanId(id)).toBe(true);
    }
  });

  it("normalizes unresolved links in the same envelope", () => {
    const withUnresolved: TopologySnapshot = {
      ...VLAN_OUT_OF_RANGE_TOPOLOGY,
      connections: [
        ...VLAN_OUT_OF_RANGE_TOPOLOGY.connections,
        {
          fromDeviceId: 40,
          toUnresolvedId: "unknown:chassis:deadbeef0001",
          fromPort: "Gi1/0/48",
          toPort: "Gi0/1",
          kind: "ethernet",
          protocol: "cdp",
          confidence: "confirmed",
          vlan: "0",
          nativeVlan: 0,
          taggedVlans: [0, 30, 4095],
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
    const handoff = envelope(withUnresolved);
    const orphan = handoff.unresolvedTopology?.connections[0];
    expect(orphan?.toUnresolvedId).toBe("unknown:chassis:deadbeef0001");
    expect(orphan).not.toHaveProperty("nativeVlan");
    expect(orphan?.taggedVlans).toEqual([30]);
    for (const id of vlanIdsIn(handoff)) {
      expect(isValidVlanId(id)).toBe(true);
    }
  });
});
