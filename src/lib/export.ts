// Export formatting for CSV, JSON and XML.
//
// The column set from v1.6 is preserved so existing spreadsheets and scripts keep
// working; the two new latency measurements are appended rather than replacing
// the `Response (ms)` column they refine.

import type { ExportFormat, HostResult } from "../types";
import type { DeviceRow } from "./live";
import { rowName } from "./live";

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
