import { describe, expect, it } from "vitest";
import type { InventoryRow } from "../types";
import type { DeviceRow } from "./live";
import {
  credentialInputError,
  emptyCredentialInput,
  endpointLabel,
  looksLikeSecretLeak,
  nameLookupFromInventory,
  speedLabel,
  summaryLine,
  targetsFromInventory,
  targetsFromScanRows,
  vlanLabel,
  type TopologyConnection,
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

  it("builds targets from scan rows without inventing device ids", () => {
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
      {
        ip: "192.168.1.50",
        mac: "AA:BB:CC:00:00:50",
        deviceId: null,
        hostname: "workstation",
        detectedName: "workstation",
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
});
