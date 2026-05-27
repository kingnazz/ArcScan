import type { Host } from "../types";

function escapeCell(value: string): string {
  // Quote when the cell contains a comma, quote, or newline; double up quotes.
  if (/[",\n\r]/.test(value)) {
    return `"${value.replace(/"/g, '""')}"`;
  }
  return value;
}

export function hostsToCsv(hosts: Host[]): string {
  const header = [
    "IP Address",
    "Hostname",
    "MAC Address",
    "Vendor",
    "Open Ports",
    "Response (ms)",
    "Status",
    "Last Seen",
    "New",
  ];

  const rows = hosts.map((h) => [
    h.ip,
    h.hostname ?? "",
    h.mac ?? "",
    h.vendor ?? "",
    h.openPorts.map((p) => `${p.port}/${p.service}`).join(" "),
    h.responseMs != null ? String(h.responseMs) : "",
    h.status,
    h.lastSeen,
    h.isNew ? "yes" : "no",
  ]);

  return [header, ...rows]
    .map((cols) => cols.map((c) => escapeCell(String(c))).join(","))
    .join("\r\n");
}

/** Trigger a browser download as a fallback when not running native save. */
export function downloadCsv(filename: string, csv: string): void {
  const blob = new Blob([csv], { type: "text/csv;charset=utf-8;" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  document.body.appendChild(link);
  link.click();
  document.body.removeChild(link);
  URL.revokeObjectURL(url);
}
