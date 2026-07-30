import { describe, expect, it } from "vitest";
import {
  columnsForWidth,
  filterRows,
  prepareRows,
  sortRows,
  visibleColumns,
  EMPTY_FILTER,
  type SortKey,
} from "./table";
import { upsertHost, type DeviceRow } from "./live";
import type { HostResult } from "../types";

function host(ip: string, overrides: Partial<HostResult> = {}): HostResult {
  return {
    ip,
    hostname: null,
    mac: null,
    vendor: null,
    open_ports: [],
    response_ms: null,
    icmp_ms: null,
    tcp_ms: null,
    ttl: null,
    os_guess: null,
    last_seen: "2026-07-01T10:00:00Z",
    ...overrides,
  };
}

function row(h: HostResult, overrides: Partial<DeviceRow> = {}): DeviceRow {
  const [built] = upsertHost([], h, false);
  return { ...built, ...overrides };
}

const network: DeviceRow[] = [
  row(
    host("10.0.0.1", {
      hostname: "gateway",
      vendor: "Ubiquiti Inc.",
      mac: "F4:92:BF:1A:0C:31",
      open_ports: [53, 80, 443],
      response_ms: 1,
      icmp_ms: 0.8,
      os_guess: "Linux/Unix/macOS",
    }),
    { status: "trusted" },
  ),
  row(
    host("10.0.0.57", {
      hostname: "front-office-printer",
      vendor: "Hewlett Packard",
      open_ports: [80, 443, 9100],
      response_ms: 9,
      icmp_ms: 8.9,
    }),
    { status: "known", change: "changed" },
  ),
  row(
    host("10.0.0.9", {
      vendor: "Intel Corporate",
      open_ports: [],
      response_ms: null,
    }),
    { change: "new" },
  ),
  row(host("10.0.0.200", { hostname: "srv-files01", open_ports: [445, 3389], response_ms: 3 })),
];

describe("filtering", () => {
  it("matches on name, address, vendor and service name", () => {
    const find = (query: string) =>
      filterRows(network, { ...EMPTY_FILTER, query }).map((r) => r.host.ip);

    expect(find("printer")).toEqual(["10.0.0.57"]);
    expect(find("10.0.0.9")).toEqual(["10.0.0.9"]);
    expect(find("ubiquiti")).toEqual(["10.0.0.1"]);
    // Service names are searchable even though the row stores port numbers.
    expect(find("rdp")).toEqual(["10.0.0.200"]);
    expect(find("9100")).toEqual(["10.0.0.57"]);
  });

  it("is case insensitive and ignores surrounding whitespace", () => {
    expect(filterRows(network, { ...EMPTY_FILTER, query: "  PRINTER " })).toHaveLength(1);
  });

  it("narrows with each additional term rather than widening", () => {
    // "443" matches two rows, "printer" one of them; together, one.
    expect(filterRows(network, { ...EMPTY_FILTER, query: "443" })).toHaveLength(2);
    expect(filterRows(network, { ...EMPTY_FILTER, query: "443 printer" })).toHaveLength(1);
  });

  it("filters to labelled devices only", () => {
    const labelled = filterRows(network, { ...EMPTY_FILTER, savedOnly: true });
    expect(labelled.map((r) => r.host.ip).sort()).toEqual(["10.0.0.1", "10.0.0.57"]);
  });

  it("filters to changed devices only", () => {
    const changed = filterRows(network, { ...EMPTY_FILTER, changesOnly: true });
    expect(changed.map((r) => r.host.ip).sort()).toEqual(["10.0.0.57", "10.0.0.9"]);
  });

  it("combines the switches with the query", () => {
    const both = filterRows(network, { query: "printer", savedOnly: true, changesOnly: true });
    expect(both.map((r) => r.host.ip)).toEqual(["10.0.0.57"]);
  });
});

describe("sorting", () => {
  it("orders addresses numerically, not as strings", () => {
    // Lexical order would put 10.0.0.200 before 10.0.0.57.
    expect(sortRows(network, "ip", "asc").map((r) => r.host.ip)).toEqual([
      "10.0.0.1",
      "10.0.0.9",
      "10.0.0.57",
      "10.0.0.200",
    ]);
    expect(sortRows(network, "ip", "desc").map((r) => r.host.ip)).toEqual([
      "10.0.0.200",
      "10.0.0.57",
      "10.0.0.9",
      "10.0.0.1",
    ]);
  });

  it("sorts missing measurements last in both directions", () => {
    // A host that never answered must not read as the fastest on the network.
    const asc = sortRows(network, "response", "asc");
    expect(asc[asc.length - 1].host.ip).toBe("10.0.0.9");
    const desc = sortRows(network, "response", "desc");
    expect(desc[0].host.ip).toBe("10.0.0.9");
  });

  it("sorts empty vendors and MACs after present ones", () => {
    const sorted = sortRows(network, "mac", "asc");
    expect(sorted[0].host.mac).toBe("F4:92:BF:1A:0C:31");
    expect(sorted[sorted.length - 1].host.mac).toBeNull();
  });

  it("puts attention-worthy states first when sorting by state", () => {
    const sorted = sortRows(network, "state", "asc").map((r) => r.host.ip);
    expect(sorted[0]).toBe("10.0.0.9"); // new
    expect(sorted[1]).toBe("10.0.0.57"); // changed
  });

  it("breaks ties by address so a streaming table never reshuffles", () => {
    // Three rows with no name at all: only the address can order them.
    const anonymous = [row(host("10.0.0.30")), row(host("10.0.0.4")), row(host("10.0.0.12"))];
    const once = sortRows(anonymous, "name", "asc").map((r) => r.host.ip);
    const twice = sortRows(sortRows(anonymous, "name", "asc"), "name", "asc").map((r) => r.host.ip);
    expect(once).toEqual(["10.0.0.4", "10.0.0.12", "10.0.0.30"]);
    expect(twice).toEqual(once);
  });

  it("does not mutate the array it is given", () => {
    const original = network.map((r) => r.host.ip);
    sortRows(network, "name", "desc");
    expect(network.map((r) => r.host.ip)).toEqual(original);
  });

  it("uses the operator's name when sorting by name", () => {
    const renamed = [
      row(host("10.0.0.2", { hostname: "zzz-host" }), { custom_name: "Alpha" }),
      row(host("10.0.0.3", { hostname: "aaa-host" })),
    ];
    expect(sortRows(renamed, "name", "asc").map((r) => r.host.ip)).toEqual([
      "10.0.0.3", // aaa-host
      "10.0.0.2", // Alpha
    ]);
  });
});

describe("column visibility", () => {
  it("drops low-priority columns as the window narrows", () => {
    const wide = columnsForWidth(1600);
    const medium = columnsForWidth(1000);
    const narrow = columnsForWidth(880);

    expect(wide).toContain("last_seen");
    expect(medium).not.toContain("last_seen");
    expect(medium).toContain("mac");
    expect(narrow).not.toContain("mac");
  });

  it("never hides the three columns a row cannot be read without", () => {
    for (const width of [320, 700, 880, 1000, 1440, 2560]) {
      const columns = columnsForWidth(width);
      expect(columns, `at ${width}px`).toEqual(expect.arrayContaining(["state", "name", "ip"]));
    }
  });

  it("honours the operator's hidden columns on top of the width rules", () => {
    const hidden: SortKey[] = ["vendor", "os"];
    const columns = visibleColumns(1600, hidden);
    expect(columns).not.toContain("vendor");
    expect(columns).not.toContain("os");
    expect(columns).toContain("mac");
  });

  it("keeps required columns even if they are somehow marked hidden", () => {
    expect(visibleColumns(1600, ["ip", "name", "state"] as SortKey[])).toEqual(
      expect.arrayContaining(["state", "name", "ip"]),
    );
  });
});

describe("prepareRows", () => {
  it("filters before sorting, so the sort only sees what is shown", () => {
    const prepared = prepareRows(network, { ...EMPTY_FILTER, query: "443" }, "ip", "asc");
    expect(prepared.map((r) => r.host.ip)).toEqual(["10.0.0.1", "10.0.0.57"]);
  });
});
