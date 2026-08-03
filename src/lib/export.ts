// Export formatting for CSV, JSON and XML.
//
// The column set from v1.6 is preserved so existing spreadsheets and scripts keep
// working; the two new latency measurements are appended rather than replacing
// the `Response (ms)` column they refine.

import type { ChangeEvent, ExportFormat, HostResult, InventoryRow } from "../types";
import type { DeviceRow } from "./live";
import { rowName } from "./live";
import { serviceWithPort } from "./format";

function csvField(value: string): string {
  if (/[",\n\r]/.test(value)) return `"${value.replace(/"/g, '""')}"`;
  return value;
}

function xmlEscape(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** The exported record for one device, in column order. */
interface ExportRecord {
  name: string;
  ip: string;
  hostname: string;
  mac: string;
  vendor: string;
  os: string;
  ttl: string;
  ports: string;
  response_ms: string;
  icmp_ms: string;
  tcp_ms: string;
  status: string;
  last_seen: string;
}

function toRecord(row: DeviceRow): ExportRecord {
  const h = row.host;
  const num = (v: number | null | undefined) => (v == null ? "" : String(v));
  return {
    name: rowName(row),
    ip: h.ip,
    hostname: h.hostname ?? "",
    mac: h.mac ?? "",
    vendor: h.vendor ?? "",
    os: h.os_guess ?? "",
    ttl: num(h.ttl),
    ports: h.open_ports.join(" "),
    response_ms: num(h.response_ms),
    icmp_ms: num(h.icmp_ms),
    tcp_ms: num(h.tcp_ms),
    status: row.status,
    last_seen: h.last_seen,
  };
}

const CSV_HEADERS = [
  "Name",
  "IP",
  "Hostname",
  "MAC",
  "Vendor",
  "OS",
  "TTL",
  "Open Ports",
  "Response (ms)",
  "ICMP (ms)",
  "TCP (ms)",
  "Status",
  "Last Seen",
];

export function buildExport(rows: DeviceRow[], format: ExportFormat): string {
  const records = rows.map(toRecord);

  if (format === "json") {
    return `${JSON.stringify(records, null, 2)}\n`;
  }

  if (format === "xml") {
    const body = records
      .map((r) => {
        const fields = Object.entries(r)
          .map(([key, value]) => `    <${key}>${xmlEscape(value)}</${key}>`)
          .join("\n");
        return `  <device>\n${fields}\n  </device>`;
      })
      .join("\n");
    return `<?xml version="1.0" encoding="UTF-8"?>\n<devices>\n${body}\n</devices>\n`;
  }

  const lines = records.map((r) => Object.values(r).map(csvField).join(","));
  return [CSV_HEADERS.join(","), ...lines].join("\n") + "\n";
}

/** Export the raw host results, for callers that have no row metadata. */
export function buildHostExport(hosts: HostResult[], format: ExportFormat): string {
  const rows: DeviceRow[] = hosts.map((host) => ({
    host,
    device_id: null,
    custom_name: null,
    status: "unclassified",
    first_seen: null,
    change: null,
    changed_fields: [],
    pending: false,
  }));
  return buildExport(rows, format);
}

export function exportFilename(target: string, format: ExportFormat): string {
  const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
  // A target made entirely of separators collapses to underscores, which is not
  // a name worth putting in a filename.
  const slug = target
    .replace(/[^a-zA-Z0-9.-]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .slice(0, 40);
  return `arcscan-${slug || "scan"}-${stamp}.${format}`;
}

// ---------------------------------------------------------------------------
// Inventory and Changes exports (v1.8)
//
// Separate record shapes from the scan export above, because they answer a
// different question: a scan export describes one moment, these describe what
// ArcScan knows across every scan. Internal identifiers are deliberately absent
// from CSV and XML; the JSON form keeps the device id, which is the only place
// a script has any use for it.
// ---------------------------------------------------------------------------

const PRESENCE_LABEL: Record<InventoryRow["presence"], string> = {
  present: "Present in latest scan",
  missing: "Missing from latest scan",
  unknown: "Unknown",
};

const STATUS_LABEL: Record<InventoryRow["status"], string> = {
  unclassified: "Unreviewed",
  known: "Known",
  trusted: "Trusted",
  watched: "Watched",
  ignored: "Ignored",
};

/** Human-readable change types, matching the words the inbox uses. */
export const CHANGE_TYPE_LABEL: Record<ChangeEvent["change_type"], string> = {
  device_added: "New device",
  device_returned: "Device returned",
  device_missing: "Device missing",
  ip_changed: "Address change",
  hostname_changed: "Hostname change",
  vendor_changed: "Manufacturer change",
  os_changed: "Operating system change",
  mac_changed: "MAC address change",
  ports_changed: "Service change",
};

const CHANGE_STATE_LABEL: Record<ChangeEvent["state"], string> = {
  unreviewed: "Unreviewed",
  acknowledged: "Acknowledged",
  ignored: "Ignored",
};

const INVENTORY_HEADERS = [
  "Network",
  "Device",
  "Status",
  "Presence",
  "Current IP",
  "Previous IPs",
  "MAC",
  "Manufacturer",
  "Hostname",
  "OS guess",
  "Open ports",
  "Open services",
  "First seen",
  "Last seen",
  "Observations",
  "Notes",
];

/** Ordered so the CSV columns and the XML element names cannot drift apart. */
function inventoryRecord(row: InventoryRow, notes: string): Record<string, string> {
  return {
    network: row.network_name ?? "",
    device: row.display_name,
    status: STATUS_LABEL[row.status] ?? row.status,
    presence: PRESENCE_LABEL[row.presence] ?? row.presence,
    current_ip: row.current_ip ?? "",
    previous_ips: row.previous_ips.join(" "),
    mac: row.mac ?? "",
    manufacturer: row.vendor ?? "",
    hostname: row.hostname ?? "",
    os_guess: row.os_guess ?? "",
    open_ports: row.open_ports.join(" "),
    open_services: row.open_ports.map(serviceWithPort).join(", "),
    first_seen: row.first_seen,
    last_seen: row.last_seen,
    observations: String(row.observation_count),
    notes,
  };
}

/**
 * Build an Inventory export.
 *
 * `notes` maps device id to note body: the inventory query deliberately does not
 * carry note text (a table only needs an indicator), so the caller fetches the
 * bodies for exactly the devices being exported.
 */
export function buildInventoryExport(
  rows: InventoryRow[],
  format: ExportFormat,
  notes: Map<number, string> = new Map(),
): string {
  const records = rows.map((row) => inventoryRecord(row, notes.get(row.device_id) ?? ""));

  if (format === "json") {
    // The id earns its place here: a script re-importing this file has nothing
    // else stable to join on when two devices share a name.
    return `${JSON.stringify(
      rows.map((row, index) => ({ device_id: row.device_id, ...records[index] })),
      null,
      2,
    )}\n`;
  }
  if (format === "xml") {
    return xmlDocument("inventory", "device", records);
  }
  return csvDocument(INVENTORY_HEADERS, records);
}

const CHANGE_HEADERS = [
  "Date",
  "Network",
  "Device",
  "IP",
  "MAC",
  "Change",
  "Previous value",
  "New value",
  "Opened ports",
  "Closed ports",
  "Scan",
  "Baseline",
  "Review state",
  "Acknowledged",
];

function changeRecord(event: ChangeEvent): Record<string, string> {
  return {
    date: event.scan_at ?? event.created_at,
    network: event.network_name ?? "",
    device: event.device_label,
    ip: event.ip ?? "",
    mac: event.mac ?? "",
    change: CHANGE_TYPE_LABEL[event.change_type] ?? event.change_type,
    previous_value: event.old_value ?? "",
    new_value: event.new_value ?? "",
    opened_ports: event.opened_ports.join(" "),
    closed_ports: event.closed_ports.join(" "),
    scan: event.scan_id == null ? "" : String(event.scan_id),
    baseline: event.baseline_scan_id == null ? "" : String(event.baseline_scan_id),
    review_state: CHANGE_STATE_LABEL[event.state] ?? event.state,
    acknowledged: event.acknowledged_at ?? "",
  };
}

/** Build a Changes export from exactly the events the caller passes in. */
export function buildChangesExport(events: ChangeEvent[], format: ExportFormat): string {
  const records = events.map(changeRecord);
  if (format === "json") {
    return `${JSON.stringify(
      events.map((event, index) => ({ event_id: event.id, ...records[index] })),
      null,
      2,
    )}\n`;
  }
  if (format === "xml") {
    return xmlDocument("changes", "change", records);
  }
  return csvDocument(CHANGE_HEADERS, records);
}

function csvDocument(headers: string[], records: Array<Record<string, string>>): string {
  const lines = records.map((r) => Object.values(r).map(csvField).join(","));
  return [headers.join(","), ...lines].join("\n") + "\n";
}

function xmlDocument(
  root: string,
  item: string,
  records: Array<Record<string, string>>,
): string {
  const body = records
    .map((r) => {
      const fields = Object.entries(r)
        .map(([key, value]) => `    <${key}>${xmlEscape(value)}</${key}>`)
        .join("\n");
      return `  <${item}>\n${fields}\n  </${item}>`;
    })
    .join("\n");
  const inner = body ? `\n${body}\n` : "\n";
  return `<?xml version="1.0" encoding="UTF-8"?>\n<${root}>${inner}</${root}>\n`;
}

/**
 * A dated filename for an Inventory or Changes export.
 *
 * The scope goes in the name (`arcscan-inventory-home-wifi-2026-08-03.csv`) so a
 * folder of exports says what each one covers without opening it.
 */
export function datedFilename(
  kind: "inventory" | "changes",
  scope: string | null,
  format: ExportFormat,
): string {
  const day = new Date().toISOString().slice(0, 10);
  const slug = (scope ?? "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40);
  return `arcscan-${kind}${slug ? `-${slug}` : ""}-${day}.${format}`;
}
