import { describe, expect, it } from "vitest";
import {
  buildChangesExport,
  buildExport,
  buildHostExport,
  buildInventoryExport,
  datedFilename,
  exportFilename,
} from "./export";
import { upsertHost, type DeviceRow } from "./live";
import type { ChangeEvent, HostResult, InventoryDiscovery, InventoryRow } from "../types";

function host(overrides: Partial<HostResult> = {}): HostResult {
  return {
    ip: "10.0.0.5",
    hostname: "nas-backup",
    mac: "AA:BB:CC:00:00:05",
    vendor: "Synology Incorporated",
    open_ports: [22, 443, 445],
    response_ms: 2,
    icmp_ms: 1.2,
    tcp_ms: 2.6,
    ttl: 64,
    os_guess: "Linux/Unix/macOS",
    last_seen: "2026-07-01T10:00:00Z",
    ...overrides,
  };
}

function row(h: HostResult, overrides: Partial<DeviceRow> = {}): DeviceRow {
  const [built] = upsertHost([], h, false);
  return { ...built, ...overrides };
}

describe("CSV export", () => {
  it("writes a header and one line per device", () => {
    const csv = buildExport([row(host()), row(host({ ip: "10.0.0.6" }))], "csv");
    const lines = csv.trimEnd().split("\n");
    expect(lines).toHaveLength(3);
    expect(lines[0]).toBe(
      "Name,IP,Hostname,MAC,Vendor,OS,TTL,Open Ports,Response (ms),ICMP (ms),TCP (ms),Status,Last Seen",
    );
    expect(lines[1]).toContain("10.0.0.5");
    expect(lines[1]).toContain("22 443 445");
  });

  it("quotes and escapes fields that would break the format", () => {
    // Vendor names really do contain commas, and a device name is operator input.
    const csv = buildExport(
      [
        row(host({ vendor: "TP-LINK TECHNOLOGIES CO.,LTD." }), {
          custom_name: 'Reception "main" desk',
        }),
      ],
      "csv",
    );
    expect(csv).toContain('"TP-LINK TECHNOLOGIES CO.,LTD."');
    expect(csv).toContain('"Reception ""main"" desk"');
  });

  it("leaves missing values empty rather than writing null", () => {
    const csv = buildExport(
      [row(host({ hostname: null, mac: null, vendor: null, os_guess: null, ttl: null }))],
      "csv",
    );
    expect(csv).not.toMatch(/null|undefined/);
  });

  it("writes only a header for an empty result set", () => {
    expect(buildExport([], "csv").trimEnd().split("\n")).toHaveLength(1);
  });

  it("uses the operator's name in the Name column", () => {
    const csv = buildExport([row(host(), { custom_name: "Backup NAS" })], "csv");
    expect(csv).toContain("Backup NAS");
  });
});

describe("JSON export", () => {
  it("produces parseable JSON with both latency measurements", () => {
    const parsed = JSON.parse(buildExport([row(host())], "json")) as Array<Record<string, string>>;
    expect(parsed).toHaveLength(1);
    expect(parsed[0].ip).toBe("10.0.0.5");
    expect(parsed[0].icmp_ms).toBe("1.2");
    expect(parsed[0].tcp_ms).toBe("2.6");
    expect(parsed[0].response_ms).toBe("2");
  });

  it("produces an empty array for no devices", () => {
    expect(JSON.parse(buildExport([], "json"))).toEqual([]);
  });
});

describe("XML export", () => {
  it("escapes markup so a hostname cannot break the document", () => {
    const xml = buildExport([row(host({ hostname: 'a<b>&"c' }))], "xml");
    expect(xml).toContain("&lt;b&gt;");
    expect(xml).toContain("&amp;");
    expect(xml).toContain("&quot;");
    expect(xml).not.toMatch(/<b>/);
  });

  it("declares the encoding and wraps the devices", () => {
    const xml = buildExport([row(host())], "xml");
    expect(xml.startsWith('<?xml version="1.0" encoding="UTF-8"?>')).toBe(true);
    expect(xml).toContain("<devices>");
    expect(xml).toContain("<device>");
  });
});

describe("host export", () => {
  it("works from raw host results with no row metadata", () => {
    const csv = buildHostExport([host()], "csv");
    // With no operator name, the hostname becomes the display name.
    expect(csv).toContain("nas-backup");
  });
});

describe("export filenames", () => {
  it("makes a target safe for a filesystem and keeps the extension", () => {
    const name = exportFilename("192.168.1.0/24", "csv");
    expect(name).toMatch(/^arcscan-192\.168\.1\.0_24-[\d-]+\.csv$/);
    expect(name).not.toContain("/");
  });

  it("falls back to a generic name for an unusable target", () => {
    expect(exportFilename("///", "json")).toMatch(/^arcscan-scan-[\d-]+\.json$/);
  });
});

// ---------------------------------------------------------------------------
// Inventory and Changes exports (v1.8)
// ---------------------------------------------------------------------------

function inventoryRow(patch: Partial<InventoryRow> = {}): InventoryRow {
  return {
    device_id: 3,
    network_scope_id: 1,
    network_name: "Home Wi-Fi",
    identity_source: "mac",
    display_name: "Office Printer",
    custom_name: "Office Printer",
    hostname: "office-printer",
    current_ip: "192.168.1.31",
    previous_ips: ["192.168.1.28", "192.168.1.20"],
    mac: "3C:D9:2B:6F:08:AA",
    vendor: "Hewlett Packard",
    os_guess: "Network device",
    status: "trusted",
    presence: "missing",
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
    ...patch,
  };
}

/** A row a credentialed Windows scan reached, with every v1.9 fact filled. */
function credentialedRow(patch: Partial<InventoryRow> = {}): InventoryRow {
  return inventoryRow({
    display_name: "WS-FINANCE-04",
    hostname: "ws-finance-04",
    vendor: "Dell Inc",
    user_device_type: null,
    physical_device_key: "uuid||4c4c454400375a108051b4c04f435331",
    physical_interface_count: 2,
    discovery: {
      detected_name: "WS-FINANCE-04",
      device_type: "workstation",
      type_confidence: "high",
      manufacturer: "Dell Inc.",
      model_name: "Latitude 7450",
      services: ["_smb._tcp"],
      sources: ["windows_credentialed"],
      last_discovered_at: "2026-08-02T09:00:00Z",
      evidence_freshness: "current",
      os_family: "Windows",
      os_product: "Windows 11",
      os_edition: "Pro",
      os_version: "24H2",
      os_build: "26100",
      os_architecture: "x64",
      windows_product_type: "1",
      hardware_manufacturer: "Dell Inc.",
      hardware_model: "Latitude 7450",
      hardware_serial: "7SZ1B43",
      system_uuid: "4C4C4544-0037-5A10-8051-B4C04F435331",
      domain: "corp.example",
      identity_evidence: ["system UUID: 4C4C4544-0037-5A10-8051-B4C04F435331"],
      identity_sources: ["windows_credentialed"],
      type_evidence: ["Windows ProductType 1 (workstation)"],
    },
    ...patch,
  });
}

/**
 * Split one CSV line into its cells, honouring quoting.
 *
 * Needed rather than `split(",")` because several exported cells legitimately
 * contain commas and are therefore quoted; splitting naively would shift every
 * column after the first such cell and quietly make a column-alignment
 * assertion test the wrong thing.
 */
function csvCells(line: string): string[] {
  const cells: string[] = [];
  let current = "";
  let quoted = false;
  for (let i = 0; i < line.length; i += 1) {
    const char = line[i];
    if (quoted) {
      if (char === '"') {
        if (line[i + 1] === '"') {
          current += '"';
          i += 1;
        } else {
          quoted = false;
        }
      } else {
        current += char;
      }
    } else if (char === '"') {
      quoted = true;
    } else if (char === ",") {
      cells.push(current);
      current = "";
    } else {
      current += char;
    }
  }
  cells.push(current);
  return cells;
}

function changeEvent(patch: Partial<ChangeEvent> = {}): ChangeEvent {
  return {
    id: 11,
    event_key: "s2|d3|ports_changed",
    scan_id: 2,
    baseline_scan_id: 1,
    network_scope_id: 1,
    network_name: "Home Wi-Fi",
    device_id: 3,
    device_label: "Home NAS",
    ip: "192.168.1.50",
    mac: "00:11:32:5D:A2:77",
    vendor: "Synology Incorporated",
    change_type: "ports_changed",
    old_value: "SSH · 22",
    new_value: "SSH · 22, HTTPS · 443",
    opened_ports: [443],
    closed_ports: [22],
    state: "acknowledged",
    created_at: "2026-08-02T09:00:00Z",
    scan_at: "2026-08-02T09:00:00Z",
    baseline_at: "2026-07-27T09:00:00Z",
    acknowledged_at: "2026-08-03T08:00:00Z",
    device_status: "trusted",
    ...patch,
  };
}

describe("inventory export", () => {
  it("writes the documented columns, in order", () => {
    const csv = buildInventoryExport([inventoryRow()], "csv");
    const lines = csv.trimEnd().split("\n");
    expect(lines[0]).toBe("Network,Device,Status,Presence,Current IP,Previous IPs,MAC,Manufacturer,Hostname,OS guess,Open ports,Open services,First seen,Last seen,Observations,Detected name,Device type,Type source,Detected type,Detected confidence,Discovery freshness,Discovered by,Detected manufacturer,Model,Advertised services,Last discovered,Notes," + "OS family,OS product,OS edition,OS version,OS build,OS architecture,Windows product type,Hardware manufacturer,Hardware model,Hardware serial,System UUID,Domain,Identity evidence,Identity source,Classification evidence,Physical device,Interfaces");
    expect(lines).toHaveLength(2);
  });

  // The v1.9 columns are appended after every v1.8 one so that a script
  // reading columns by position keeps working. This is the test that says so,
  // and the reason the new columns are not grouped with the discovery columns
  // they belong with.
  it("keeps every v1.8 column at the position it had", () => {
    const csv = buildInventoryExport([inventoryRow()], "csv");
    const headers = csvCells(csv.trimEnd().split("\n")[0]);
    const v18 = "Network,Device,Status,Presence,Current IP,Previous IPs,MAC,Manufacturer,Hostname,OS guess,Open ports,Open services,First seen,Last seen,Observations,Detected name,Device type,Type source,Detected type,Detected confidence,Discovery freshness,Discovered by,Detected manufacturer,Model,Advertised services,Last discovered,Notes".split(",");
    expect(headers.slice(0, v18.length)).toEqual(v18);
  });

  it("leaves the v1.9 columns blank on a row no deep scan reached", () => {
    // Blank, never "Unknown": a blank cell says "not established", where the
    // word reads as an answer.
    const csv = buildInventoryExport([inventoryRow()], "csv");
    const headers = csvCells(csv.trimEnd().split("\n")[0]);
    const values = csvCells(csv.trimEnd().split("\n")[1]);
    const v19 = "OS family,OS product,OS edition,OS version,OS build,OS architecture,Windows product type,Hardware manufacturer,Hardware model,Hardware serial,System UUID,Domain,Identity evidence,Identity source,Classification evidence,Physical device,Interfaces".split(",");
    for (const column of v19) {
      const index = headers.indexOf(column);
      expect(index).toBeGreaterThan(-1);
      expect(values[index] ?? "").toBe("");
    }
  });

  it("spells presence and status out rather than exporting internal words", () => {
    const csv = buildInventoryExport([inventoryRow()], "csv");
    expect(csv).toContain("Missing from latest scan");
    expect(csv).toContain("Trusted");
    // Never the raw enum value.
    expect(csv).not.toContain(",missing,");
    expect(csv).not.toContain(",unclassified,");
  });

  it("carries the credentialed Windows facts into the CSV", () => {
    const csv = buildInventoryExport([credentialedRow()], "csv");
    const headers = csvCells(csv.trimEnd().split("\n")[0]);
    const values = csvCells(csv.trimEnd().split("\n")[1]);
    const cell = (column: string) => values[headers.indexOf(column)];

    expect(cell("OS product")).toBe("Windows 11");
    expect(cell("OS edition")).toBe("Pro");
    expect(cell("OS version")).toBe("24H2");
    expect(cell("OS build")).toBe("26100");
    expect(cell("OS architecture")).toBe("x64");
    expect(cell("Hardware model")).toBe("Latitude 7450");
    expect(cell("Hardware serial")).toBe("7SZ1B43");
    expect(cell("System UUID")).toBe("4C4C4544-0037-5A10-8051-B4C04F435331");
    expect(cell("Domain")).toBe("corp.example");
    expect(cell("Interfaces")).toBe("2");
  });

  it("writes the Windows product type as both the number and the word", () => {
    // A script wants the 1; a person reading the spreadsheet wants the word.
    const csv = buildInventoryExport([credentialedRow()], "csv");
    expect(csv).toContain("1 (workstation)");

    const server = buildInventoryExport(
      [
        credentialedRow({
          discovery: {
            ...credentialedRow().discovery!,
            windows_product_type: "3",
            device_type: "server",
          },
        }),
      ],
      "csv",
    );
    expect(server).toContain("3 (server)");
  });

  it("exports the new device types with the words the interface uses", () => {
    for (const [type, label] of [
      ["workstation", "Workstation"],
      ["server", "Server"],
      ["domain_controller", "Domain controller"],
      ["switch", "Switch"],
      ["access_point", "Access point"],
      ["firewall", "Firewall"],
      ["management_controller", "Management controller"],
    ] as const) {
      const csv = buildInventoryExport(
        [credentialedRow({ discovery: { ...credentialedRow().discovery!, device_type: type } })],
        "csv",
      );
      expect(csv).toContain(label);
    }
  });

  it("carries the new facts into JSON and XML too", () => {
    const json = JSON.parse(buildInventoryExport([credentialedRow()], "json"));
    expect(json[0].os_product).toBe("Windows 11");
    expect(json[0].windows_product_type).toBe("1 (workstation)");
    expect(json[0].system_uuid).toBe("4C4C4544-0037-5A10-8051-B4C04F435331");
    expect(json[0].device_id).toBe(3);

    const xml = buildInventoryExport([credentialedRow()], "xml");
    expect(xml).toContain("<os_product>Windows 11</os_product>");
    expect(xml).toContain("<hardware_serial>7SZ1B43</hardware_serial>");
    // Element names are the record keys, so they stay valid XML names.
    expect(xml).not.toMatch(/<[a-z_]* /);
  });

  it("says why a device is called what it is", () => {
    const csv = buildInventoryExport([credentialedRow()], "csv");
    expect(csv).toContain("Windows ProductType 1 (workstation)");
  });

  it("groups two interfaces of one machine under one physical device", () => {
    const rows = [
      credentialedRow({ device_id: 3, current_ip: "10.0.0.5" }),
      credentialedRow({ device_id: 4, current_ip: "10.0.1.5" }),
    ];
    const json = JSON.parse(buildInventoryExport(rows, "json"));
    // Both rows survive with their own address, and both name the same box.
    expect(json).toHaveLength(2);
    expect(json[0].physical_device).toBe(json[1].physical_device);
    expect(json[0].physical_device).not.toBe("");
    expect(json[0].current_ip).not.toBe(json[1].current_ip);
  });

  it("maps the unclassified status to the word the interface uses", () => {
    const csv = buildInventoryExport([inventoryRow({ status: "unclassified" })], "csv");
    expect(csv).toContain("Unreviewed");
  });

  it("carries every previous address and both port forms", () => {
    const csv = buildInventoryExport([inventoryRow()], "csv");
    expect(csv).toContain("192.168.1.28 192.168.1.20");
    expect(csv).toContain("80 443");
    expect(csv).toContain("HTTP · 80, HTTPS · 443");
  });

  it("includes note bodies the caller fetched, and nothing when there are none", () => {
    const withNotes = buildInventoryExport(
      [inventoryRow()],
      "csv",
      new Map([[3, "Toner reordered automatically"]]),
    );
    expect(withNotes).toContain("Toner reordered automatically");
    expect(buildInventoryExport([inventoryRow()], "csv")).not.toContain("Toner reordered");
  });

  it("keeps internal ids out of CSV and XML, and only in JSON", () => {
    expect(buildInventoryExport([inventoryRow()], "csv")).not.toContain("device_id");
    expect(buildInventoryExport([inventoryRow()], "xml")).not.toContain("device_id");
    const json = JSON.parse(buildInventoryExport([inventoryRow()], "json"));
    expect(json[0].device_id).toBe(3);
    expect(json[0].presence).toBe("Missing from latest scan");
  });

  it("produces well-formed XML with one element per device", () => {
    const xml = buildInventoryExport([inventoryRow(), inventoryRow({ device_id: 4 })], "xml");
    expect(xml.startsWith('<?xml version="1.0" encoding="UTF-8"?>')).toBe(true);
    expect(xml).toContain("<inventory>");
    expect(xml.match(/<device>/g)).toHaveLength(2);
  });

  it("writes an empty but valid document when nothing is selected", () => {
    expect(buildInventoryExport([], "csv").trimEnd().split("\n")).toHaveLength(1);
    expect(buildInventoryExport([], "xml")).toContain("</inventory>");
    expect(JSON.parse(buildInventoryExport([], "json"))).toEqual([]);
  });
});

describe("changes export", () => {
  it("writes the documented columns, in order", () => {
    const csv = buildChangesExport([changeEvent()], "csv");
    expect(csv.trimEnd().split("\n")[0]).toBe(
      "Date,Network,Device,IP,MAC,Change,Previous value,New value,Opened ports,Closed ports,Scan,Baseline,Review state,Acknowledged",
    );
  });

  it("carries the review state, the acknowledgement date and both scans", () => {
    const csv = buildChangesExport([changeEvent()], "csv");
    expect(csv).toContain("Acknowledged");
    expect(csv).toContain("2026-08-03T08:00:00Z");
    expect(csv).toContain("Service change");
    expect(csv).toContain("443");
    expect(csv).toContain("22");
  });

  it("exports exactly the events it is given, ignored ones included", () => {
    const events = [changeEvent({ id: 1 }), changeEvent({ id: 2, state: "ignored" })];
    const csv = buildChangesExport(events, "csv");
    expect(csv.trimEnd().split("\n")).toHaveLength(3);
    expect(csv).toContain("Ignored");
  });

  it("leaves a pruned scan's columns blank rather than inventing an id", () => {
    const csv = buildChangesExport([changeEvent({ scan_id: null, baseline_scan_id: null })], "csv");
    expect(csv).toContain(",,,Acknowledged");
  });
});

describe("dated filenames", () => {
  it("names the kind and the day", () => {
    const name = datedFilename("inventory", null, "csv");
    expect(name).toMatch(/^arcscan-inventory-\d{4}-\d{2}-\d{2}\.csv$/);
    expect(datedFilename("changes", null, "json")).toMatch(
      /^arcscan-changes-\d{4}-\d{2}-\d{2}\.json$/,
    );
  });

  it("carries the network so a folder of exports explains itself", () => {
    expect(datedFilename("inventory", "Home Wi-Fi", "csv")).toMatch(
      /^arcscan-inventory-home-wi-fi-\d{4}-\d{2}-\d{2}\.csv$/,
    );
  });

  it("collapses a name made entirely of punctuation instead of leaving dashes", () => {
    expect(datedFilename("inventory", "///", "csv")).toMatch(
      /^arcscan-inventory-\d{4}-\d{2}-\d{2}\.csv$/,
    );
  });
});

describe("the export distinguishes a correction from a detection", () => {
  const detected = (patch: Partial<InventoryDiscovery> = {}): InventoryDiscovery => ({
    detected_name: "Living Room",
    device_type: "media_device",
    type_confidence: "medium",
    manufacturer: "Example Corp",
    model_name: "TV-123",
    services: ["_airplay._tcp"],
    sources: ["mdns"],
    last_discovered_at: "2026-08-05T09:00:00Z",
    evidence_freshness: "current",
    ...patch,
  });

  it("writes four type columns rather than one", () => {
    const csv = buildInventoryExport(
      [inventoryRow({ discovery: detected(), user_device_type: "printer" })],
      "csv",
    );
    const values = csv.split("\n")[1];
    // "Printer" alone cannot say whether ArcScan worked it out or a person
    // corrected it, and a spreadsheet full of unattributable types is worse
    // than one that says.
    expect(values).toContain("Printer");
    expect(values).toContain("User");
    expect(values).toContain("Media device");
    expect(values).toContain("medium");
  });

  it("marks an automatic type as automatic", () => {
    const values = buildInventoryExport(
      [
        inventoryRow({
          discovery: detected({ device_type: "printer", type_confidence: "high" }),
          user_device_type: null,
        }),
      ],
      "csv",
    ).split("\n")[1];
    expect(values).toContain("Automatic");
    expect(values).not.toContain("User");
  });

  it("leaves the detected columns blank where no discovery-capable scan reached the device", () => {
    const json = JSON.parse(
      buildInventoryExport([inventoryRow({ discovery: null, user_device_type: null })], "json"),
    )[0];
    expect(json.detected_type).toBe("");
    expect(json.detected_confidence).toBe("");
    expect(json.discovery_freshness).toBe("");
    // A blank cell says "not established"; the word would read as an answer.
    expect(json.device_type).toBe("Unknown");
    expect(json.type_source).toBe("Automatic");
  });

  it("carries the freshness state and not the stale evidence itself", () => {
    const json = JSON.parse(
      buildInventoryExport(
        [inventoryRow({ discovery: detected({ evidence_freshness: "stale" }) })],
        "json",
      ),
    )[0];
    expect(json.discovery_freshness).toBe("stale");
    // The drawer shows the rows; a CSV full of them would let one long-lived
    // device dominate the file.
    expect(Object.keys(json)).not.toContain("stale_evidence");
  });
});
