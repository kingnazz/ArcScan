import { describe, expect, it } from "vitest";
import { buildInventoryExport } from "./export";
import { EMPTY_INVENTORY_FILTER, prepareInventory } from "./inventory";
import {
  DISCONNECTED_CONNECTION,
  HandoffAttempt,
  PORTABLE_SESSION_COPY,
  SEND_EXPLANATION,
  buildHandoffEnvelope,
  canSendSingleNetwork,
  createFallbackHandoffId,
  createHandoffId,
  destinationLabel,
  displayTokenPrefix,
  handoffRowsForNetwork,
  nextModeOnSend,
  parseArcAtlasError,
  presenceCopyIsConservative,
  sanitizeUserMessage,
  selectedNetworkName,
  sendConfirmation,
  successCounts,
} from "./arcatlas";
import type { InventoryDiscovery, InventoryRow } from "../types";

function discovery(patch: Partial<InventoryDiscovery> = {}): InventoryDiscovery {
  return {
    detected_name: "Office Printer",
    device_type: "printer",
    type_confidence: "high",
    evidence_freshness: "fresh",
    sources: ["mdns"],
    manufacturer: "HP",
    model_name: "LaserJet",
    services: ["ipp"],
    last_discovered_at: "2026-08-02T09:00:00Z",
    ...patch,
  };
}

function inventoryRow(patch: Partial<InventoryRow> = {}): InventoryRow {
  return {
    device_id: 3,
    network_scope_id: 1,
    network_name: "192.168.10.0/24",
    identity_source: "mac",
    display_name: "Office Printer",
    custom_name: "Office Printer",
    hostname: "office-printer",
    current_ip: "192.168.10.31",
    previous_ips: ["192.168.10.28"],
    mac: "3C:D9:2B:6F:08:AA",
    vendor: "Hewlett Packard",
    os_guess: "Network device",
    status: "trusted",
    presence: "present",
    first_seen: "2026-07-01T09:00:00Z",
    last_seen: "2026-08-02T09:00:00Z",
    last_completed_scan_id: 4,
    last_completed_scan_at: "2026-08-02T09:00:00Z",
    observation_count: 6,
    open_ports: [80, 443],
    notes_present: true,
    notes_excerpt: "Toner reordered",
    latest_response_ms: 9,
    latest_icmp_ms: 8.9,
    latest_tcp_ms: 10.3,
    discovery: discovery(),
    user_device_type: null,
    ...patch,
  };
}

describe("send action routing", () => {
  it("opens connection setup when disconnected", () => {
    expect(nextModeOnSend(DISCONNECTED_CONNECTION, true)).toBe("connect");
  });

  it("does not send just because a scan finished", () => {
    expect(nextModeOnSend(DISCONNECTED_CONNECTION, true)).not.toBe("confirm");
  });
});

describe("single-network send", () => {
  it("blocks send when Inventory is showing more than one network", () => {
    const rows = [
      inventoryRow(),
      inventoryRow({ device_id: 4, network_scope_id: 2, network_name: "Guest" }),
    ];
    expect(canSendSingleNetwork({ networkId: null, networkCount: 2, rows })).toBe(false);
  });

  it("allows send after one network is chosen", () => {
    const rows = [inventoryRow(), inventoryRow({ device_id: 4 })];
    expect(canSendSingleNetwork({ networkId: 1, networkCount: 2, rows })).toBe(true);
    expect(selectedNetworkName(rows, 1, [{ id: 1, name: "192.168.10.0/24" }])).toBe("192.168.10.0/24");
  });
});

describe("full selected-network snapshot", () => {
  const raw: InventoryRow[] = [
    inventoryRow({ device_id: 1, presence: "present", display_name: "Office Printer" }),
    inventoryRow({
      device_id: 2,
      presence: "missing",
      display_name: "Spare Switch",
      custom_name: "Spare Switch",
      hostname: "spare-switch",
      discovery: discovery({ device_type: "switch", detected_name: "Spare Switch" }),
    }),
    inventoryRow({
      device_id: 3,
      presence: "unknown",
      display_name: "Unknown Camera",
      custom_name: "Unknown Camera",
      hostname: "unknown-camera",
      discovery: discovery({ device_type: "camera", detected_name: "Unknown Camera" }),
    }),
    inventoryRow({
      device_id: 4,
      network_scope_id: 2,
      network_name: "Guest",
      display_name: "Guest AP",
      presence: "present",
      discovery: discovery({ device_type: "access-point", detected_name: "Guest AP" }),
    }),
  ];

  it("View = Present still sends Present + Missing + Unknown rows from selected network", () => {
    const filtered = prepareInventory(
      raw,
      { ...EMPTY_INVENTORY_FILTER, view: "present", networkId: 1 },
      "device",
      "asc",
    );
    const snapshot = handoffRowsForNetwork({ rows: raw, networkId: 1, networkCount: 2 });
    expect(filtered.map((row) => row.presence)).toEqual(["present"]);
    expect(snapshot.map((row) => row.presence).sort()).toEqual(["missing", "present", "unknown"]);
    expect(snapshot).toHaveLength(3);
  });

  it("search text does not reduce ArcAtlas snapshot", () => {
    const filtered = prepareInventory(
      raw,
      { ...EMPTY_INVENTORY_FILTER, query: "Office Printer", networkId: 1 },
      "device",
      "asc",
    );
    const snapshot = handoffRowsForNetwork({ rows: raw, networkId: 1, networkCount: 2 });
    expect(filtered).toHaveLength(1);
    expect(snapshot).toHaveLength(3);
  });

  it("device type filter does not reduce ArcAtlas snapshot", () => {
    const filtered = prepareInventory(
      raw,
      { ...EMPTY_INVENTORY_FILTER, deviceType: "printer", networkId: 1 },
      "device",
      "asc",
    );
    const snapshot = handoffRowsForNetwork({ rows: raw, networkId: 1, networkCount: 2 });
    expect(filtered.every((row) => row.discovery?.device_type === "printer")).toBe(true);
    expect(filtered.length).toBeLessThan(snapshot.length);
    expect(snapshot).toHaveLength(3);
  });

  it("selected network excludes every other network", () => {
    const snapshot = handoffRowsForNetwork({ rows: raw, networkId: 1, networkCount: 2 });
    expect(snapshot.every((row) => row.network_scope_id === 1)).toBe(true);
    expect(snapshot.some((row) => row.network_scope_id === 2)).toBe(false);
  });

  it("confirmation count equals full selected-network row count", () => {
    const snapshot = handoffRowsForNetwork({ rows: raw, networkId: 1, networkCount: 2 });
    const confirmation = sendConfirmation({
      connection: {
        ...DISCONNECTED_CONNECTION,
        configured: true,
        clientName: "Cedar Ridge",
        siteName: "Seattle HQ",
      },
      networkName: "192.168.10.0/24",
      deviceCount: snapshot.length,
    });
    expect(confirmation.deviceCount).toBe(3);
    expect(confirmation.deviceCount).toBe(raw.filter((row) => row.network_scope_id === 1).length);
  });
});

describe("confirmation", () => {
  it("shows destination, network and device count", () => {
    const confirmation = sendConfirmation({
      connection: {
        ...DISCONNECTED_CONNECTION,
        configured: true,
        clientName: "Cedar Ridge",
        siteName: "Seattle HQ",
      },
      networkName: "192.168.10.0/24",
      deviceCount: 42,
    });
    expect(confirmation.destination).toBe("Cedar Ridge / Seattle HQ");
    expect(confirmation.networkName).toBe("192.168.10.0/24");
    expect(confirmation.deviceCount).toBe(42);
    expect(confirmation.explanation).toBe(SEND_EXPLANATION);
  });
});

describe("exporter reuse", () => {
  it("uses the exact Inventory JSON row shape for the handoff inventory", () => {
    const rows = [inventoryRow()];
    const notes = new Map([[3, "Keep the spare toner upstairs."]]);
    const exported = JSON.parse(buildInventoryExport(rows, "json", notes));
    const envelope = buildHandoffEnvelope({
      rows,
      notes,
      networkName: "192.168.10.0/24",
      handoffId: "11111111-1111-4111-8111-111111111111",
      generatedAt: "2026-09-01T12:00:00.000Z",
      sourceVersion: "1.8.4",
    });
    expect(envelope.schemaVersion).toBe(1);
    expect(envelope.inventory).toEqual(exported);
    expect(envelope.inventory[0]).toMatchObject({
      device_id: 3,
      network: "192.168.10.0/24",
      device_name: "Office Printer",
      manufacturer: "Hewlett Packard",
      hostname: "office-printer",
      current_ip: "192.168.10.31",
      mac: "3C:D9:2B:6F:08:AA",
      os_guess: "Network device",
      open_ports: "80 443",
      presence: "Present in latest scan",
      model: "LaserJet",
      notes: "Keep the spare toner upstairs.",
    });
  });
});

describe("handoff ids", () => {
  const uuidV4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

  it("reuses the same id on retry and issues a new one after success", () => {
    const ids = ["aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"];
    let index = 0;
    const attempt = new HandoffAttempt();
    const first = attempt.begin(() => ids[index++]!);
    expect(attempt.begin(() => ids[index++]!)).toBe(first);
    attempt.failRetryable();
    expect(attempt.begin(() => ids[index++]!)).toBe(first);
    attempt.succeed();
    expect(attempt.begin(() => ids[index++]!)).toBe(ids[1]);
  });

  it("fallback-generated ids are distinct UUID v4 values", () => {
    const first = createFallbackHandoffId();
    const second = createFallbackHandoffId();
    expect(first).toMatch(uuidV4);
    expect(second).toMatch(uuidV4);
    expect(first).not.toBe(second);
    expect(first).not.toBe("00000000-0000-4000-8000-000000000000");
    expect(createHandoffId()).toMatch(uuidV4);
  });
});

describe("errors and copy", () => {
  it("maps 401 to a reconfigure error", () => {
    const error = parseArcAtlasError(
      '{"code":"unauthorized","message":"The ArcAtlas connection token is invalid or revoked."}',
    );
    expect(error.code).toBe("unauthorized");
    expect(error.retryable).toBe(false);
  });

  it("treats timeouts as retryable", () => {
    const error = parseArcAtlasError('{"code":"timeout","message":"The ArcAtlas request timed out."}');
    expect(error.code).toBe("timeout");
    expect(error.retryable).toBe(true);
  });

  it("never puts the stored token into user-facing copy", () => {
    const cleaned = sanitizeUserMessage("Bearer atlas_arcscan_supersecret failed");
    expect(cleaned).not.toContain("supersecret");
  });

  it("uses conservative presence words", () => {
    const copy = ["Observed: 42", "Present: 40", "Not observed: 1", "Unknown: 1", SEND_EXPLANATION].join("\n");
    expect(presenceCopyIsConservative(copy)).toBe(true);
    expect(presenceCopyIsConservative("Device is offline")).toBe(false);
    expect(PORTABLE_SESSION_COPY).toContain("session only");
  });

  it("renders success counts with not-observed wording", () => {
    expect(
      successCounts({
        runId: "run-1",
        recordCount: 42,
        presentCount: 40,
        missingCount: 1,
        unknownCount: 1,
        clientName: "Cedar Ridge",
        siteName: "Seattle HQ",
        discoveryUrl: "https://atlas.example.com/discovery?run=run-1",
        duplicate: false,
        status: 201,
      }),
    ).toEqual({ observed: 42, present: 40, notObserved: 1, unknown: 1 });
  });

  it("shows only a token prefix, never a full token", () => {
    expect(displayTokenPrefix("atlas_arcscan_abcd")).toBe("atlas_arcscan_abcd...");
    expect(destinationLabel({ clientName: "Cedar Ridge", siteName: "Seattle HQ" })).toBe(
      "Cedar Ridge / Seattle HQ",
    );
  });
});
