import { describe, expect, it } from "vitest";
import {
  EMPTY_CHANGE_FILTER,
  actionsFor,
  changeTimestamp,
  describeChange,
  filterChanges,
  groupChanges,
  matchesChangeView,
  type ChangeFilter,
} from "./changes";
import type { ChangeEvent } from "../types";

let nextId = 1;

function event(patch: Partial<ChangeEvent> = {}): ChangeEvent {
  return {
    id: nextId++,
    event_key: `s2|d1|${patch.change_type ?? "device_added"}`,
    scan_id: 2,
    baseline_scan_id: 1,
    network_scope_id: 1,
    network_name: "Home Wi-Fi",
    device_id: 1,
    device_label: "Living Room TV",
    ip: "192.168.1.44",
    mac: "FC:65:DE:19:2D:6B",
    vendor: "Samsung Electronics",
    change_type: "device_added",
    old_value: null,
    new_value: "192.168.1.44",
    opened_ports: [],
    closed_ports: [],
    state: "unreviewed",
    created_at: "2026-08-02T09:00:00Z",
    scan_at: "2026-08-02T09:00:00Z",
    baseline_at: "2026-07-27T09:00:00Z",
    acknowledged_at: null,
    device_status: "unclassified",
    ...patch,
  };
}

const filter = (patch: Partial<ChangeFilter> = {}): ChangeFilter => ({
  ...EMPTY_CHANGE_FILTER,
  ...patch,
});

describe("change views", () => {
  it("keeps ignored entries out of every view except Ignored", () => {
    const ignored = event({ state: "ignored" });
    for (const view of ["all", "unreviewed", "acknowledged", "added"] as const) {
      expect(matchesChangeView(ignored, view)).toBe(false);
    }
    expect(matchesChangeView(ignored, "ignored")).toBe(true);
  });

  it("routes each change type to the filter that describes it", () => {
    const cases: Array<[ChangeEvent["change_type"], ChangeFilter["view"]]> = [
      ["device_added", "added"],
      ["device_missing", "missing"],
      ["device_returned", "returned"],
      ["ip_changed", "address"],
      ["mac_changed", "address"],
      ["hostname_changed", "name"],
      ["vendor_changed", "name"],
      ["os_changed", "name"],
      ["ports_changed", "services"],
    ];
    for (const [type, view] of cases) {
      expect(matchesChangeView(event({ change_type: type }), view)).toBe(true);
    }
  });

  it("defaults to the unreviewed inbox", () => {
    expect(EMPTY_CHANGE_FILTER.view).toBe("unreviewed");
    const events = [event(), event({ state: "acknowledged" })];
    expect(filterChanges(events, filter())).toHaveLength(1);
    expect(filterChanges(events, filter({ view: "all" }))).toHaveLength(2);
  });
});

describe("change filtering", () => {
  const now = Date.parse("2026-08-03T12:00:00Z");

  it("applies the time window to when the scan ran", () => {
    const events = [
      event({ scan_at: "2026-08-02T09:00:00Z" }),
      event({ scan_at: "2026-06-01T09:00:00Z" }),
    ];
    expect(filterChanges(events, filter({ window: "7d" }), now)).toHaveLength(1);
    expect(filterChanges(events, filter({ window: "all" }), now)).toHaveLength(2);
  });

  it("keeps an entry whose timestamp cannot be parsed rather than hiding it", () => {
    const events = [event({ scan_at: "not a date", created_at: "not a date" })];
    expect(filterChanges(events, filter({ window: "7d" }), now)).toHaveLength(1);
  });

  it("falls back to when the change was recorded if the scan date is gone", () => {
    expect(changeTimestamp(event({ scan_at: null }))).toBe("2026-08-02T09:00:00Z");
  });

  it("searches device names, addresses, networks and services", () => {
    const events = [
      event({ device_label: "Office Printer" }),
      event({
        device_label: "Home NAS",
        change_type: "ports_changed",
        opened_ports: [443],
        closed_ports: [22],
      }),
    ];
    expect(filterChanges(events, filter({ view: "all", query: "printer" }), now)).toHaveLength(1);
    expect(filterChanges(events, filter({ view: "all", query: "https" }), now)).toHaveLength(1);
    expect(filterChanges(events, filter({ view: "all", query: "home wi-fi" }), now)).toHaveLength(2);
  });

  it("filters by network", () => {
    const events = [event(), event({ network_scope_id: 2, network_name: "Office" })];
    expect(filterChanges(events, filter({ view: "all", networkId: 2 }), now)).toHaveLength(1);
  });

  it("shows a change that arrives while a filter is set, if it matches", () => {
    const before = [event({ device_label: "Office Printer" })];
    const shown = filterChanges(before, filter({ query: "printer" }), now);
    expect(shown).toHaveLength(1);
    // A new unreviewed entry for the same device appears without touching the
    // filter, because filtering is a pure function of the current list.
    const after = [...before, event({ device_label: "Office Printer", change_type: "ip_changed" })];
    expect(filterChanges(after, filter({ query: "printer" }), now)).toHaveLength(2);
  });
});

describe("grouping", () => {
  it("puts one device's changes from one scan together", () => {
    const events = [
      event({ device_id: 7, change_type: "ip_changed" }),
      event({ device_id: 7, change_type: "ports_changed" }),
      event({ device_id: 9, change_type: "device_added" }),
    ];
    const groups = groupChanges(events);
    expect(groups).toHaveLength(2);
    expect(groups[0].events).toHaveLength(2);
    expect(groups[1].events).toHaveLength(1);
  });

  it("keeps the same device apart across different scans", () => {
    const events = [
      event({ device_id: 7, scan_id: 2 }),
      event({ device_id: 7, scan_id: 3 }),
    ];
    expect(groupChanges(events)).toHaveLength(2);
  });

  it("groups an unidentified device by address so it is not merged with others", () => {
    const events = [
      event({ device_id: null, ip: "10.0.0.1" }),
      event({ device_id: null, ip: "10.0.0.2" }),
    ];
    expect(groupChanges(events)).toHaveLength(2);
  });
});

describe("change descriptions", () => {
  it("renders opened and closed services as structured lists", () => {
    const text = describeChange(
      event({ change_type: "ports_changed", opened_ports: [443], closed_ports: [22] }),
    );
    expect(text).toContain("Opened: HTTPS · 443");
    expect(text).toContain("Closed: SSH · 22");
  });

  it("never claims a missing device is offline", () => {
    const text = describeChange(event({ change_type: "device_missing" }));
    expect(text).toBe("Did not answer this scan");
    expect(text.toLowerCase()).not.toContain("offline");
  });

  it("shows old and new values for a plain field change", () => {
    expect(
      describeChange(
        event({ change_type: "ip_changed", old_value: "10.0.0.4", new_value: "10.0.0.9" }),
      ),
    ).toBe("10.0.0.4 → 10.0.0.9");
  });
});

describe("actions", () => {
  it("offers Trust and Rename only on a new device that is not already trusted", () => {
    expect(actionsFor(event({ change_type: "device_added" }))).toContain("trust");
    expect(actionsFor(event({ change_type: "device_added" }))).toContain("rename");
    expect(
      actionsFor(event({ change_type: "device_added", device_status: "trusted" })),
    ).not.toContain("trust");
    expect(actionsFor(event({ change_type: "ports_changed" }))).not.toContain("trust");
  });

  it("swaps Acknowledge for Reopen once an entry has been reviewed", () => {
    expect(actionsFor(event({ state: "unreviewed" }))).toContain("acknowledge");
    expect(actionsFor(event({ state: "acknowledged" }))).toContain("reopen");
    expect(actionsFor(event({ state: "acknowledged" }))).not.toContain("acknowledge");
  });

  it("drops Ignore once the device is already ignored", () => {
    expect(actionsFor(event({ device_status: "ignored" }))).not.toContain("ignore");
    expect(actionsFor(event({ state: "ignored" }))).not.toContain("ignore");
  });

  it("offers no device actions for an entry whose device is gone", () => {
    const orphan = actionsFor(event({ device_id: null }));
    expect(orphan).not.toContain("review");
    expect(orphan).not.toContain("ignore");
    // Acknowledging the record itself still works.
    expect(orphan).toContain("acknowledge");
  });
});
