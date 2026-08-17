import { describe, expect, it } from "vitest";
import {
  EMPTY_INVENTORY_FILTER,
  filterInventory,
  inventoryHaystack,
  inventoryHeadline,
  matchesDeviceType,
  matchesView,
  prepareInventory,
  presentDeviceTypes,
  sortInventory,
  visibleInventoryColumns,
  type InventoryFilter,
} from "./inventory";
import type { InventoryDiscovery, InventoryRow } from "../types";

/** Discovery facts for a row, defaulting to a confidently-typed printer. */
function discovery(patch: Partial<InventoryDiscovery> = {}): InventoryDiscovery {
  return {
    detected_name: "Acme LaserFast 400",
    device_type: "printer",
    type_confidence: "high",
    manufacturer: "Hewlett Packard",
    model_name: "LaserFast 400",
    services: ["_ipp._tcp"],
    sources: ["mdns", "ssdp"],
    last_discovered_at: "2026-08-05T09:00:00Z",
    ...patch,
  };
}

function row(patch: Partial<InventoryRow> = {}): InventoryRow {
  return {
    device_id: 1,
    network_scope_id: 1,
    network_name: "Home Wi-Fi",
    identity_source: "mac",
    display_name: "Office Printer",
    custom_name: "Office Printer",
    hostname: "office-printer",
    current_ip: "192.168.1.31",
    previous_ips: ["192.168.1.28"],
    mac: "3C:D9:2B:6F:08:AA",
    vendor: "Hewlett Packard",
    os_guess: "Network device",
    status: "known",
    presence: "present",
    first_seen: "2026-07-01T09:00:00Z",
    last_seen: "2026-08-02T09:00:00Z",
    last_completed_scan_id: 4,
    last_completed_scan_at: "2026-08-02T09:00:00Z",
    observation_count: 6,
    open_ports: [80, 443, 9100],
    notes_present: true,
    notes_excerpt: "Toner reordered automatically",
    latest_response_ms: 9,
    latest_icmp_ms: 8.9,
    latest_tcp_ms: 10.3,
    ...patch,
  };
}

const filter = (patch: Partial<InventoryFilter> = {}): InventoryFilter => ({
  ...EMPTY_INVENTORY_FILTER,
  ...patch,
});

describe("inventory search", () => {
  const rows = [
    row(),
    row({
      device_id: 2,
      display_name: "Home NAS",
      custom_name: "Home NAS",
      hostname: "nas-home",
      current_ip: "192.168.1.50",
      previous_ips: [],
      mac: "00:11:32:5D:A2:77",
      vendor: "Synology Incorporated",
      open_ports: [22, 445],
      notes_excerpt: null,
      notes_present: false,
    }),
  ];

  it("is case-insensitive and matches partial words", () => {
    expect(filterInventory(rows, filter({ query: "PRINT" }))).toHaveLength(1);
    expect(filterInventory(rows, filter({ query: "synology" }))).toHaveLength(1);
  });

  it("narrows rather than widens when several terms are given", () => {
    expect(filterInventory(rows, filter({ query: "printer 9100" }))).toHaveLength(1);
    // Both terms must match the same row.
    expect(filterInventory(rows, filter({ query: "printer synology" }))).toHaveLength(0);
  });

  it("reaches previous addresses, service names, networks and note text", () => {
    expect(filterInventory(rows, filter({ query: "192.168.1.28" }))).toHaveLength(1);
    expect(filterInventory(rows, filter({ query: "https" }))).toHaveLength(1);
    expect(filterInventory(rows, filter({ query: "home wi-fi" }))).toHaveLength(2);
    expect(filterInventory(rows, filter({ query: "toner" }))).toHaveLength(1);
  });

  it("builds a haystack that carries every searchable field", () => {
    const hay = inventoryHaystack(row());
    for (const needle of [
      "office printer",
      "192.168.1.31",
      "192.168.1.28",
      "3c:d9:2b:6f:08:aa",
      "hewlett packard",
      "home wi-fi",
      "toner",
    ]) {
      expect(hay).toContain(needle);
    }
  });
});

describe("inventory views", () => {
  it("keeps observed presence separate from what the operator decided", () => {
    // A trusted device that has gone missing must appear under both, because
    // collapsing the two would make one of the facts unreachable.
    const missingTrusted = row({ presence: "missing", status: "trusted" });
    expect(matchesView(missingTrusted, "missing")).toBe(true);
    expect(matchesView(missingTrusted, "trusted")).toBe(true);
    expect(matchesView(missingTrusted, "present")).toBe(false);
    expect(matchesView(missingTrusted, "unreviewed")).toBe(false);
  });

  it("maps each view to exactly the right rows", () => {
    const rows = [
      row({ device_id: 1, presence: "present", status: "trusted" }),
      row({ device_id: 2, presence: "missing", status: "unclassified" }),
      row({ device_id: 3, presence: "unknown", status: "ignored" }),
    ];
    const ids = (view: InventoryFilter["view"]) =>
      filterInventory(rows, filter({ view })).map((r) => r.device_id);

    expect(ids("all")).toEqual([1, 2, 3]);
    expect(ids("present")).toEqual([1]);
    expect(ids("missing")).toEqual([2]);
    expect(ids("unknown")).toEqual([3]);
    expect(ids("trusted")).toEqual([1]);
    expect(ids("unreviewed")).toEqual([2]);
    expect(ids("ignored")).toEqual([3]);
  });

  it("filters by network without touching the other filters", () => {
    const rows = [
      row({ device_id: 1, network_scope_id: 1 }),
      row({ device_id: 2, network_scope_id: 2, network_name: "Office" }),
    ];
    expect(filterInventory(rows, filter({ networkId: 2 })).map((r) => r.device_id)).toEqual([2]);
    expect(filterInventory(rows, filter({ networkId: null }))).toHaveLength(2);
  });
});

describe("inventory sorting", () => {
  it("orders addresses numerically, not as text", () => {
    const rows = [
      row({ device_id: 1, current_ip: "192.168.1.120" }),
      row({ device_id: 2, current_ip: "192.168.1.9" }),
    ];
    expect(sortInventory(rows, "address", "asc").map((r) => r.current_ip)).toEqual([
      "192.168.1.9",
      "192.168.1.120",
    ]);
  });

  it("sorts attention-worthy presence first", () => {
    const rows = [
      row({ device_id: 1, presence: "present" }),
      row({ device_id: 2, presence: "unknown" }),
      row({ device_id: 3, presence: "missing" }),
    ];
    expect(sortInventory(rows, "status", "asc").map((r) => r.presence)).toEqual([
      "missing",
      "unknown",
      "present",
    ]);
  });

  it("puts devices with no measurement last in both directions", () => {
    const rows = [
      row({ device_id: 1, latest_response_ms: null, current_ip: "10.0.0.1" }),
      row({ device_id: 2, latest_response_ms: 4, current_ip: "10.0.0.2" }),
    ];
    expect(sortInventory(rows, "response", "asc").map((r) => r.device_id)).toEqual([2, 1]);
    expect(sortInventory(rows, "response", "desc").map((r) => r.device_id)).toEqual([1, 2]);
  });

  it("is stable: equal values never shuffle between renders", () => {
    const rows = [
      row({ device_id: 5, display_name: "Same", current_ip: "10.0.0.5" }),
      row({ device_id: 2, display_name: "Same", current_ip: "10.0.0.5" }),
    ];
    const once = sortInventory(rows, "device", "asc").map((r) => r.device_id);
    const twice = sortInventory(rows.slice().reverse(), "device", "asc").map((r) => r.device_id);
    expect(once).toEqual(twice);
  });

  it("filters before it sorts", () => {
    const rows = [
      row({ device_id: 1, presence: "missing", current_ip: "10.0.0.9" }),
      row({ device_id: 2, presence: "present", current_ip: "10.0.0.1" }),
    ];
    const out = prepareInventory(rows, filter({ view: "missing" }), "address", "asc");
    expect(out.map((r) => r.device_id)).toEqual([1]);
  });
});

describe("inventory columns", () => {
  it("never hides Device or Address, whatever the width", () => {
    for (const width of [700, 900, 1024, 1280, 1440]) {
      const columns = visibleInventoryColumns(width, [], true);
      expect(columns).toContain("device");
      expect(columns).toContain("address");
    }
  });

  it("drops lower-priority columns as the window narrows", () => {
    const wide = visibleInventoryColumns(1440, [], true);
    const narrow = visibleInventoryColumns(880, [], true);
    expect(wide.length).toBeGreaterThan(narrow.length);
    // Status and Last seen stay readable at the narrow size.
    expect(narrow).toContain("status");
    expect(narrow).toContain("last_seen");
  });

  it("hides the Network column for someone with a single network", () => {
    expect(visibleInventoryColumns(1440, [], false)).not.toContain("network");
    expect(visibleInventoryColumns(1440, [], true)).toContain("network");
  });

  it("shows an optional column only once it has been turned on", () => {
    expect(visibleInventoryColumns(1440, [], true)).not.toContain("mac");
    expect(visibleInventoryColumns(1440, ["mac"], true)).toContain("mac");
  });
});

describe("inventory headline", () => {
  it("reads as one compact line", () => {
    expect(inventoryHeadline({ total: 86, present: 62, missing: 9, unknown: 15 })).toBe(
      "86 devices · 62 present · 9 missing · 15 unknown",
    );
  });

  it("leaves out states with nothing in them", () => {
    expect(inventoryHeadline({ total: 1, present: 1, missing: 0, unknown: 0 })).toBe(
      "1 device · 1 present",
    );
  });
});


describe("discovery in the inventory", () => {
  it("finds a device by its detected name, model or type", () => {
    const rows = [
      row({ device_id: 1, discovery: discovery() }),
      row({
        device_id: 2,
        display_name: "Study Desktop",
        custom_name: "Study Desktop",
        hostname: "desktop-study",
        vendor: "Dell Inc.",
        discovery: null,
      }),
    ];
    const find = (query: string) =>
      filterInventory(rows, { ...EMPTY_INVENTORY_FILTER, query }).map((r) => r.device_id);

    // Each of these appears only in the printer's discovery record.
    expect(find("laserfast")).toEqual([1]);
    expect(find("printer")).toEqual([1]);
    expect(find("hewlett")).toEqual([1]);
  });

  it("finds a device by a service, however the searcher spells it", () => {
    const rows = [
      row({ device_id: 1, discovery: discovery({ services: ["_ipp._tcp"] }) }),
      row({
        device_id: 2,
        display_name: "Study Desktop",
        custom_name: "Study Desktop",
        hostname: "desktop-study",
        vendor: "Dell Inc.",
        open_ports: [22],
        discovery: discovery({
          services: ["_ssh._tcp"],
          detected_name: null,
          device_type: "computer",
          manufacturer: "Dell Inc.",
          model_name: null,
        }),
      }),
    ];
    const find = (query: string) =>
      filterInventory(rows, { ...EMPTY_INVENTORY_FILTER, query }).map((r) => r.device_id);

    expect(find("_ipp")).toEqual([1]);
    expect(find("ipp printing")).toEqual([1]);
    expect(find("ssh")).toEqual([2]);
  });

  it("narrows to one device type, and treats undiscovered devices as unknown", () => {
    const rows = [
      row({ device_id: 1, discovery: discovery() }),
      row({ device_id: 2, discovery: discovery({ device_type: "camera" }) }),
      row({ device_id: 3, discovery: null }),
    ];
    const ofType = (deviceType: string | null) =>
      filterInventory(rows, { ...EMPTY_INVENTORY_FILTER, deviceType }).map((r) => r.device_id);

    expect(ofType("printer")).toEqual([1]);
    expect(ofType("camera")).toEqual([2]);
    // A device no discovery-capable scan reached answers the same question as
    // one discovery could not type: "what does ArcScan not recognise?"
    expect(ofType("unknown")).toEqual([3]);
    expect(ofType(null)).toEqual([1, 2, 3]);
  });

  it("combines the type filter with the search and the presence view", () => {
    const rows = [
      row({ device_id: 1, presence: "present", discovery: discovery() }),
      row({ device_id: 2, presence: "missing", discovery: discovery() }),
    ];
    const filter: InventoryFilter = {
      query: "laserfast",
      view: "missing",
      networkId: null,
      deviceType: "printer",
    };
    expect(filterInventory(rows, filter).map((r) => r.device_id)).toEqual([2]);
  });

  it("offers only the types actually present, with Unknown last", () => {
    const rows = [
      row({ device_id: 1, discovery: discovery({ device_type: "television" }) }),
      row({ device_id: 2, discovery: null }),
      row({ device_id: 3, discovery: discovery({ device_type: "camera" }) }),
      row({ device_id: 4, discovery: discovery({ device_type: "camera" }) }),
    ];
    expect(presentDeviceTypes(rows)).toEqual(["camera", "television", "unknown"]);
  });

  it("matches a device against a type directly", () => {
    expect(matchesDeviceType(row({ discovery: discovery() }), "printer")).toBe(true);
    expect(matchesDeviceType(row({ discovery: discovery() }), "camera")).toBe(false);
    expect(matchesDeviceType(row({ discovery: null }), "unknown")).toBe(true);
  });

  it("sorts by type, detected name and model without crashing on missing data", () => {
    const rows = [
      row({ device_id: 1, discovery: discovery({ device_type: "television" }) }),
      row({ device_id: 2, discovery: null }),
      row({ device_id: 3, discovery: discovery({ device_type: "camera" }) }),
    ];
    for (const key of ["type", "detected_name", "model", "last_discovered"] as const) {
      const sorted = sortInventory(rows, key, "asc");
      expect(sorted).toHaveLength(3);
      // Rows with nothing to sort on go last rather than first in both
      // directions, so an empty column never leads the table.
      expect(sortInventory(rows, key, "desc")).toHaveLength(3);
    }
  });

  it("leaves the haystack unchanged for a device discovery never reached", () => {
    const plain = inventoryHaystack(row({ discovery: null }));
    expect(plain).toContain("office printer");
    expect(plain).not.toContain("laserfast");
  });

  it("keeps the discovery columns off unless they are turned on", () => {
    const wide = visibleInventoryColumns(1600, [], false);
    expect(wide).not.toContain("type");
    expect(wide).not.toContain("detected_name");

    const chosen = visibleInventoryColumns(1600, ["type", "model"], false);
    expect(chosen).toContain("type");
    expect(chosen).toContain("model");
    expect(chosen).not.toContain("detected_name");
  });
});
