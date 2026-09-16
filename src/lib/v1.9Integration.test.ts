import { describe, expect, it } from "vitest";
import { buildHandoffEnvelope, type ArcAtlasHandoffV2 } from "./arcatlas";
import { buildInventoryExport } from "./export";
import {
  SNMP_AUTH_PASSWORD_FIXTURE,
  SNMP_COMMUNITY_FIXTURE,
  SNMP_PRIV_PASSWORD_FIXTURE,
  V19_INTEGRATION_ROWS,
  V19_INTEGRATION_TOPOLOGY,
  WINDOWS_PASSWORD_FIXTURE,
} from "./fixtures/v1.9Integration";

/** The endpoint and uniqueness checks performed by ArcAtlas-Next PR #13. */
function assertArcAtlas13ParserAssumptions(envelope: ArcAtlasHandoffV2): void {
  const inventory = envelope.inventory as Array<Record<string, unknown>>;
  const counts = new Map<string, number>();
  for (const row of inventory) {
    const id = String(row.device_id);
    counts.set(id, (counts.get(id) ?? 0) + 1);
  }
  expect([...counts.values()].every((count) => count === 1)).toBe(true);
  for (const connection of envelope.topology.connections) {
    expect(counts.get(String(connection.fromDeviceId))).toBe(1);
    expect(counts.get(String(connection.toDeviceId))).toBe(1);
    expect(connection).not.toHaveProperty("fromUnresolvedId");
    expect(connection).not.toHaveProperty("toUnresolvedId");
  }
}

describe("ArcScan v1.9 integrated ArcAtlas contract", () => {
  it("builds schema v2 from the real exporter without collapsing physical-device rows", () => {
    const notes = new Map([[30, "Managed out-of-band controller"]]);
    const envelope = buildHandoffEnvelope({
      rows: V19_INTEGRATION_ROWS,
      notes,
      networkName: "Site LAN",
      handoffId: "00000000-0000-4000-8000-000000000019",
      sourceVersion: "1.8.7",
      generatedAt: "2026-09-16T12:01:00.000Z",
      topology: V19_INTEGRATION_TOPOLOGY,
    });

    expect(envelope.schemaVersion).toBe(2);
    if (envelope.schemaVersion !== 2) throw new Error("expected schema v2");

    const actualExporterRows = JSON.parse(
      buildInventoryExport(V19_INTEGRATION_ROWS, "json", notes),
    ) as Array<Record<string, unknown>>;
    expect(envelope.inventory).toEqual(actualExporterRows);
    expect(envelope.inventory).toHaveLength(5);

    const ids = actualExporterRows.map((row) => row.device_id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).toEqual([21, 22, 30, 31, 40]);

    const firstTransport = actualExporterRows.find((row) => row.device_id === 21);
    const secondTransport = actualExporterRows.find((row) => row.device_id === 22);
    expect(firstTransport?.physical_device).toBe(secondTransport?.physical_device);
    expect(secondTransport).toMatchObject({
      os_family: "Windows",
      os_product: "Windows Server 2022",
      os_edition: "Standard",
      os_build: "20348",
      hardware_serial: "J7K2M13",
      system_uuid: "4C4C4544-004A-3710-8054-B7C04F324D13",
      windows_product_type: "3 (server)",
    });
    expect(actualExporterRows.find((row) => row.device_id === 30)?.device_type).toBe(
      "Management controller",
    );

    const transportLink = envelope.topology.connections.find(
      (connection) => connection.toDeviceId === 22,
    );
    expect(transportLink).toMatchObject({
      fromDeviceId: 40,
      toDeviceId: 22,
      confidence: "confirmed",
      protocol: "lldp",
      vlan: "20",
      nativeVlan: 20,
    });
    const poeLink = envelope.topology.connections.find(
      (connection) => connection.toDeviceId === 31,
    );
    expect(poeLink).toMatchObject({
      confidence: "strong",
      protocol: "fdb",
      speedMbps: 2500,
      vlan: "trunk",
      nativeVlan: 10,
      taggedVlans: [10, 20, 30],
      poe: { enabled: true, watts: 13.7 },
    });

    expect(envelope.topology.connections).toHaveLength(2);
    expect(envelope.unresolvedTopology?.connections).toHaveLength(1);
    expect(envelope.unresolvedTopology?.unknownNodes).toHaveLength(1);
    expect(
      envelope.topology.connections.some(
        (connection) =>
          "fromUnresolvedId" in connection || "toUnresolvedId" in connection,
      ),
    ).toBe(false);

    assertArcAtlas13ParserAssumptions(envelope);

    const serialized = JSON.stringify(envelope);
    for (const secret of [
      WINDOWS_PASSWORD_FIXTURE,
      SNMP_COMMUNITY_FIXTURE,
      SNMP_AUTH_PASSWORD_FIXTURE,
      SNMP_PRIV_PASSWORD_FIXTURE,
    ]) {
      expect(serialized).not.toContain(secret);
    }
    expect(serialized).not.toMatch(/"(?:password|community|credential)"\s*:/i);
  });

  it("refuses a schema v2 payload whose transport ids are not unique", () => {
    expect(() =>
      buildHandoffEnvelope({
        rows: [V19_INTEGRATION_ROWS[0]!, V19_INTEGRATION_ROWS[0]!],
        notes: new Map(),
        networkName: "Site LAN",
        handoffId: "00000000-0000-4000-8000-000000000020",
        topology: V19_INTEGRATION_TOPOLOGY,
      }),
    ).toThrow(/unique local device_id/i);
  });
});
