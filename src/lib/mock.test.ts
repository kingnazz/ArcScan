// Behaviour tests against the browser demo backend.
//
// The mock is the only backend the browser tests, the screenshots and the site
// demo ever see, so the rules it mirrors have to be the real ones. These tests
// pin the behaviour that would be embarrassing to get wrong in a screenshot: a
// stopped scan marking devices missing, presence claimed without a completed
// scan, or a rename that does not reach the inbox.
//
// The mock holds module-level state seeded at import, so this file owns it and
// the assertions run in order.

import { describe, expect, it } from "vitest";
import { mock } from "./mock";
import type { InventoryRow } from "../types";

function find(rows: InventoryRow[], name: string): InventoryRow {
  const row = rows.find((r) => r.display_name === name);
  if (!row) throw new Error(`no inventory row named ${name}: ${rows.map((r) => r.display_name)}`);
  return row;
}

describe("the demo inventory", () => {
  it("covers both networks and keeps their devices apart", () => {
    const summary = mock.inventory();
    expect(summary.networks.map((n) => n.name).sort()).toEqual(["Home Wi-Fi", "Office"]);
    const home = summary.rows.filter((r) => r.network_name === "Home Wi-Fi");
    const office = summary.rows.filter((r) => r.network_name === "Office");
    expect(home.length).toBeGreaterThan(5);
    expect(office.length).toBeGreaterThan(5);
    expect(home.length + office.length).toBe(summary.rows.length);
  });

  it("shows all three presence states, so none of them is untested by accident", () => {
    const summary = mock.inventory();
    expect(summary.present).toBeGreaterThan(0);
    expect(summary.missing).toBeGreaterThan(0);
    expect(summary.unknown).toBeGreaterThan(0);
    expect(summary.present + summary.missing + summary.unknown).toBe(summary.rows.length);
    expect(summary.needs_completed_scan).toBe(false);
  });

  it("marks the tablet missing and never calls it offline", () => {
    const tablet = find(mock.inventory().rows, "Kitchen Tablet");
    expect(tablet.presence).toBe("missing");
    // Its history is intact: being missing removes nothing.
    expect(tablet.observation_count).toBeGreaterThan(0);
    expect(tablet.first_seen).toBeTruthy();
  });

  it("leaves a device seen only under different coverage as Unknown", () => {
    // The office one-off was seen by a wide scan and never by one with the
    // coverage the office's latest scan used, so its absence proves nothing.
    const unknown = mock.inventory().rows.filter((r) => r.presence === "unknown");
    expect(unknown).toHaveLength(1);
    expect(unknown[0].network_name).toBe("Office");
  });

  it("is deliberately imperfect: some devices resolve no name at all", () => {
    const rows = mock.inventory().rows;
    expect(rows.some((r) => r.hostname == null)).toBe(true);
    expect(rows.some((r) => r.vendor == null)).toBe(true);
    expect(rows.some((r) => r.custom_name == null)).toBe(true);
  });

  it("carries notes as an indicator and a searchable excerpt, not a full body", () => {
    const printer = find(mock.inventory().rows, "Office Printer");
    expect(printer.notes_present).toBe(true);
    expect(printer.notes_excerpt).toContain("Toner");
    expect(printer.notes_excerpt!.length).toBeLessThanOrEqual(160);
  });

  it("records an address change as previous addresses on one device", () => {
    const printer = find(mock.inventory().rows, "Office Printer");
    expect(printer.current_ip).toBe("192.168.1.31");
    expect(printer.previous_ips).toContain("192.168.1.28");
    expect(printer.observation_count).toBe(2);
  });
});

describe("the demo changes inbox", () => {
  it("holds every change type the release describes", () => {
    const types = new Set(mock.changeEvents().events.map((e) => e.change_type));
    for (const expected of [
      "device_added",
      "device_missing",
      "ip_changed",
      "hostname_changed",
      "ports_changed",
    ] as const) {
      expect(types).toContain(expected);
    }
  });

  it("keeps port changes as numbers, not only as display text", () => {
    const ports = mock.changeEvents().events.find((e) => e.change_type === "ports_changed");
    expect(ports?.opened_ports.length).toBeGreaterThan(0);
  });

  it("records an ignored device's changes without putting them in the inbox", () => {
    const feed = mock.changeEvents();
    const camera = feed.events.filter((e) => e.device_label === "Driveway Camera");
    expect(camera.length).toBeGreaterThan(0);
    expect(camera.every((e) => e.state === "ignored")).toBe(true);
    expect(feed.events.filter((e) => e.state === "unreviewed")).toHaveLength(feed.unreviewed);
  });

  it("names the scan and the baseline on every entry", () => {
    for (const event of mock.changeEvents().events) {
      expect(event.scan_id).not.toBeNull();
      expect(event.baseline_scan_id).not.toBeNull();
      expect(event.scan_at).toBeTruthy();
      expect(event.baseline_at).toBeTruthy();
    }
  });
});

describe("a stopped scan", () => {
  it("keeps what it found, marks nothing missing and creates no changes", async () => {
    const before = mock.inventory();
    const beforeEvents = mock.changeEvents().total;

    // Start a scan and stop it a moment later, exactly as pressing Stop does.
    const running = mock.scan(
      {
        target: "192.168.1.0/24",
        ports: [80, 443],
        timeout_ms: 900,
        concurrency: 64,
        tcp_concurrency: 256,
        ping_concurrency: 32,
        profile: "quick-lan",
        arp_assist: null,
      },
      {},
    );
    await new Promise((resolve) => setTimeout(resolve, 250));
    mock.cancelScan();
    const result = await running;
    expect(result.cancelled).toBe(true);
    expect(result.hosts.length).toBeLessThan(before.rows.length);
    mock.save(result);

    const after = mock.inventory();
    // Presence still comes from the last completed scan, so nothing moved.
    expect(after.missing).toBe(before.missing);
    expect(after.present).toBe(before.present);
    expect(mock.changeEvents().total).toBe(beforeEvents);

    // The partial scan is saved and labelled, not discarded.
    const newest = mock.listScans()[0];
    expect(newest.status).toBe("cancelled");
    expect(newest.missing_count).toBe(0);
    expect(mock.compareScan(newest.id).baseline_scan_id).toBeNull();
  }, 20_000);
});

describe("renames and classifications propagate", () => {
  it("takes a device rename to the inventory and the inbox at once", () => {
    const tv = find(mock.inventory().rows, "Living Room TV");
    mock.setDeviceName(tv.device_id, "Lounge TV");

    expect(find(mock.inventory().rows, "Lounge TV").device_id).toBe(tv.device_id);
    const events = mock.changeEvents().events.filter((e) => e.device_id === tv.device_id);
    expect(events.length).toBeGreaterThan(0);
    expect(events.every((e) => e.device_label === "Lounge TV")).toBe(true);

    mock.setDeviceName(tv.device_id, "Living Room TV");
  });

  it("takes a network rename to the inventory, the inbox and history", () => {
    mock.renameNetworkScope(2, "Workshop");
    expect(mock.inventory().networks.some((n) => n.name === "Workshop")).toBe(true);
    expect(mock.changeEvents().events.some((e) => e.network_name === "Workshop")).toBe(true);
    expect(mock.listScans().some((s) => s.scope_name === "Workshop")).toBe(true);
    mock.renameNetworkScope(2, "Office");
  });

  it("hides an ignored device's unreviewed changes and brings them back on undo", () => {
    const nas = find(mock.inventory().rows, "Office NAS");
    const before = mock.changeEvents().unreviewed;

    const outcome = mock.setDeviceStatuses([nas.device_id], "ignored");
    expect(outcome.updated).toBe(1);
    expect(outcome.missing).toEqual([]);

    const feed = mock.changeEvents();
    expect(feed.unreviewed).toBeLessThan(before);
    // Nothing was deleted: the entries are still there, filtered out.
    expect(feed.events.filter((e) => e.device_id === nas.device_id).length).toBeGreaterThan(0);
    expect(find(mock.inventory().rows, "Office NAS").status).toBe("ignored");

    mock.setDeviceStatuses([nas.device_id], "trusted");
  });

  it("reports ids that no longer exist rather than failing silently", () => {
    const outcome = mock.setDeviceStatuses([999_999], "trusted");
    expect(outcome.updated).toBe(0);
    expect(outcome.missing).toEqual([999_999]);
  });
});

describe("acknowledging", () => {
  it("preserves the entry, stamps the time and can be undone", () => {
    const target = mock.changeEvents().events.find((e) => e.state === "unreviewed");
    expect(target).toBeDefined();
    const total = mock.changeEvents().total;

    mock.setChangeState([target!.id], "acknowledged");
    let after = mock.changeEvents().events.find((e) => e.id === target!.id)!;
    expect(after.state).toBe("acknowledged");
    expect(after.acknowledged_at).toBeTruthy();
    expect(mock.changeEvents().total).toBe(total);

    mock.setChangeState([target!.id], "unreviewed");
    after = mock.changeEvents().events.find((e) => e.id === target!.id)!;
    expect(after.state).toBe("unreviewed");
    expect(after.acknowledged_at).toBeNull();
  });
});

describe("the device drawer's data", () => {
  it("carries presence, network and the persisted changes for one device", () => {
    const printer = find(mock.inventory().rows, "Office Printer");
    const detail = mock.deviceDetail(printer.device_id);
    expect(detail.presence).toBe(printer.presence);
    expect(detail.network_name).toBe("Home Wi-Fi");
    expect(detail.events.length).toBeGreaterThan(0);
    expect(detail.previous_ips).toContain("192.168.1.28");
    expect(detail.device.notes).toBeTruthy();
  });

  it("returns note bodies only for the devices an export asks for", () => {
    const rows = mock.inventory().rows;
    const withNotes = rows.filter((r) => r.notes_present).map((r) => r.device_id);
    const notes = mock.deviceNotes(withNotes);
    expect(notes.length).toBe(withNotes.length);
    expect(mock.deviceNotes([])).toEqual([]);
  });
});

describe("deleting a scan", () => {
  it("keeps the inventory and the change records readable", () => {
    const scans = mock.listScans();
    const oldest = scans[scans.length - 1];
    const devicesBefore = mock.inventory().rows.length;
    const eventsBefore = mock.changeEvents().total;

    mock.deleteScan(oldest.id);

    expect(mock.inventory().rows.length).toBe(devicesBefore);
    expect(mock.changeEvents().total).toBe(eventsBefore);
  });
});


describe("demo discovery", () => {
  it("gives the printer a detected name the operator's name still overrides", () => {
    const printer = mock
      .inventory()
      .rows.find((r) => r.current_ip === "192.168.1.31");
    expect(printer).toBeDefined();
    // The operator named it, so that is what shows.
    expect(printer?.display_name).toBe("Office Printer");
    // The detected name is kept alongside, not thrown away.
    expect(printer?.discovery?.detected_name).toBe("Acme LaserFast 400");
    expect(printer?.discovery?.device_type).toBe("printer");
    expect(printer?.discovery?.type_confidence).toBe("high");
  });

  it("types the router, the TV and the camera from what they advertise", () => {
    const rows = mock.inventory().rows;
    const typeOf = (ip: string) =>
      rows.find((r) => r.current_ip === ip)?.discovery?.device_type;
    expect(typeOf("192.168.1.1")).toBe("router");
    expect(typeOf("192.168.1.60")).toBe("camera");
    expect(typeOf("192.168.1.50")).toBe("nas");
    // The television is the demo's user-override case: ArcScan reads it as a
    // media device, and the *detected* value is what this asserts. What the
    // interface shows for it is covered by the effective-type tests.
    expect(typeOf("192.168.1.44")).toBe("media_device");
  });

  it("ships an Auto device, a corrected one and an explicit Unknown", () => {
    const rows = mock.inventory().rows;
    const rowFor = (ip: string) => rows.find((r) => r.current_ip === ip);
    // Auto: no correction, so ArcScan's own answer stands.
    expect(rowFor("192.168.1.1")?.user_device_type).toBeNull();
    // Corrected: the operator overruled a medium-confidence media device.
    expect(rowFor("192.168.1.44")?.user_device_type).toBe("television");
    expect(rowFor("192.168.1.44")?.discovery?.device_type).toBe("media_device");
    // Explicit Unknown, which is an answer and not the absence of one.
    expect(rowFor("192.168.1.60")?.user_device_type).toBe("unknown");
    expect(rowFor("192.168.1.60")?.discovery?.device_type).toBe("camera");
  });

  it("shows evidence that is current, getting old and stale", () => {
    const rows = mock.inventory().rows;
    const freshnessOf = (ip: string) =>
      rows.find((r) => r.current_ip === ip)?.discovery?.evidence_freshness;
    expect(freshnessOf("192.168.1.31")).toBe("current");
    expect(freshnessOf("192.168.1.77")).toBe("aging");
    expect(freshnessOf("192.168.1.81")).toBe("stale");
  });

  it("reduces a high-confidence type whose evidence has all gone stale", () => {
    const row = mock.inventory().rows.find((r) => r.current_ip === "192.168.1.81");
    // The classifier said high; nothing has confirmed it in three qualifying
    // scans, so what is shown is medium. The type itself does not move.
    expect(row?.discovery?.device_type).toBe("media_device");
    expect(row?.discovery?.type_confidence).toBe("medium");
    const detail = mock.deviceDetail(row!.device_id);
    expect(detail.discovery?.raw_type_confidence).toBe("high");
    expect(detail.discovery?.type_confidence).toBe("medium");
  });

  it("keeps stale evidence on file rather than deleting it", () => {
    const row = mock.inventory().rows.find((r) => r.current_ip === "192.168.1.81");
    const detail = mock.deviceDetail(row!.device_id);
    const stale = detail.discovery?.evidence.filter((e) => e.freshness === "stale") ?? [];
    expect(stale.length).toBeGreaterThan(0);
    expect(stale.some((e) => e.value === "MediaServer")).toBe(true);
    // Dated in scans, which is the only count ArcScan actually observed.
    expect(stale.every((e) => e.misses >= 3)).toBe(true);
  });

  it("leaves a device that advertises nothing without a discovery record", () => {
    const rows = mock.inventory().rows;
    // The Windows desktop resolves reverse DNS and nothing else.
    const desktop = rows.find((r) => r.current_ip === "192.168.1.15");
    expect(desktop?.discovery).toBeNull();
    // As does the wholly unidentified device.
    const unknown = rows.find((r) => r.current_ip === "192.168.1.88");
    expect(unknown?.discovery).toBeNull();
  });

  it("down-ranks a device that advertises only its category", () => {
    const rows = mock.inventory().rows;
    const speaker = rows.find((r) => r.current_ip === "192.168.1.23");
    expect(speaker?.discovery?.detected_name).toBe("speaker");
    // "speaker" is generic, so it does not become the device's name.
    expect(speaker?.display_name).not.toBe("speaker");
    expect(speaker?.discovery?.type_confidence).toBe("low");
  });

  it("gives the drawer the evidence behind what it shows", () => {
    const printer = mock
      .inventory()
      .rows.find((r) => r.current_ip === "192.168.1.31");
    const detail = mock.deviceDetail(printer!.device_id);
    const record = detail.discovery;
    expect(record).toBeTruthy();
    expect(record?.type_evidence.length).toBeGreaterThan(0);
    expect(record?.evidence.some((e) => e.kind === "service")).toBe(true);
    expect(record?.evidence.every((e) => e.first_seen !== "")).toBe(true);
    // A second name it advertised is offered rather than silently dropped.
    expect(record?.alternate_names).toContain("Acme LaserFast 400 (Office)");
  });

  it("records what the discovery pass managed with each scan", () => {
    const scans = mock.listScans();
    const completed = scans.find((s) => s.status === "completed");
    expect(completed?.discovery_mode).toBe("full");

    // A stopped scan never claims a full pass, whatever it managed to hear:
    // the phases after Stop did not run, and that is what gates change events.
    const cancelled = scans.find((s) => s.status === "cancelled");
    expect(cancelled?.discovery_mode).toBe("none");
  });
});
