// Results-table sorting, filtering and column definitions.
//
// Pure so the behaviour that matters most (a stable order while rows stream in,
// and a filter that searches what the operator can see) is testable without
// rendering anything.

import type { DeviceRow } from "./live";
import { ipToNum, serviceLabel } from "./format";
import { rowName } from "./live";

export type SortKey =
  | "state"
  | "name"
  | "ip"
  | "mac"
  | "vendor"
  | "os"
  | "ports"
  | "response"
  | "last_seen";

export type SortDir = "asc" | "desc";

export interface ColumnDef {
  key: SortKey;
  label: string;
  /** Columns are hidden from the narrowest window up, lowest priority first. */
  priority: 1 | 2 | 3;
  align?: "right";
  /** Never hidden and never turned off: without these a row means nothing. */
  required?: boolean;
}

/**
 * Column order and priority.
 *
 * Name, address and state are the anchors, so they are required. MAC, vendor,
 * OS, response and last-seen are supporting detail and drop away as the window
 * narrows, in reverse priority order.
 */
export const COLUMNS: ColumnDef[] = [
  { key: "state", label: "State", priority: 1, required: true },
  { key: "name", label: "Name", priority: 1, required: true },
  { key: "ip", label: "IP address", priority: 1, required: true },
  { key: "ports", label: "Open services", priority: 1 },
  { key: "vendor", label: "Manufacturer", priority: 2 },
  { key: "mac", label: "MAC address", priority: 2 },
  { key: "os", label: "OS", priority: 3 },
  { key: "response", label: "Response", priority: 3, align: "right" },
  { key: "last_seen", label: "Last seen", priority: 3, align: "right" },
];

export const OPTIONAL_COLUMNS = COLUMNS.filter((c) => !c.required);

/**
 * Which columns fit a given window width.
 *
 * Below roughly 1,100px the low-priority detail goes first, then the
 * second-priority identifiers. The table still scrolls horizontally, so nothing
 * becomes unreachable; hiding is about what is worth showing by default.
 */
export function columnsForWidth(width: number): SortKey[] {
  const maxPriority = width < 900 ? 1 : width < 1100 ? 2 : 3;
  return COLUMNS.filter((c) => c.required || c.priority <= maxPriority).map((c) => c.key);
}

/** Visible columns: what fits, intersected with what the operator turned on. */
export function visibleColumns(width: number, hidden: SortKey[]): SortKey[] {
  const fits = new Set(columnsForWidth(width));
  return COLUMNS.filter((c) => c.required || (fits.has(c.key) && !hidden.includes(c.key))).map(
    (c) => c.key,
  );
}

/** Everything a filter query is matched against, lowercased. */
export function searchHaystack(row: DeviceRow): string {
  const { host } = row;
  return [
    rowName(row),
    host.ip,
    host.hostname ?? "",
    host.mac ?? "",
    host.vendor ?? "",
    host.os_guess ?? "",
    row.status,
    row.change ?? "",
    host.open_ports.join(" "),
    host.open_ports.map(serviceLabel).join(" "),
  ]
    .join(" ")
    .toLowerCase();
}

export interface TableFilter {
  query: string;
  /** Show only devices marked known, trusted or watched. */
  savedOnly: boolean;
  /** Show only new, returned or changed devices. */
  changesOnly: boolean;
}

export const EMPTY_FILTER: TableFilter = { query: "", savedOnly: false, changesOnly: false };

export function filterRows(rows: DeviceRow[], filter: TableFilter): DeviceRow[] {
  const query = filter.query.trim().toLowerCase();
  // Every term must match, so "printer 443" narrows rather than widens.
  const terms = query ? query.split(/\s+/) : [];
  return rows.filter((row) => {
    if (filter.savedOnly && row.status === "unclassified") return false;
    if (filter.changesOnly && row.change == null) return false;
    if (terms.length === 0) return true;
    const hay = searchHaystack(row);
    return terms.every((term) => hay.includes(term));
  });
}

/** Rank for the state column: attention-worthy states sort to the top. */
function stateRank(row: DeviceRow): number {
  if (row.change === "new") return 0;
  if (row.change === "returned") return 1;
  if (row.change === "changed") return 2;
  if (row.status === "watched") return 3;
  if (row.status === "trusted" || row.status === "known") return 4;
  return 5;
}

export function sortRows(rows: DeviceRow[], key: SortKey, dir: SortDir): DeviceRow[] {
  const factor = dir === "asc" ? 1 : -1;
  // A copy, because the caller's array is React state.
  return rows.slice().sort((a, b) => {
    const primary = compareBy(a, b, key);
    // Address is the tiebreak for every column, so equal values never shuffle
    // between renders while a scan streams.
    if (primary !== 0) return primary * factor;
    return (ipToNum(a.host.ip) - ipToNum(b.host.ip)) * (key === "ip" ? factor : 1);
  });
}

function compareBy(a: DeviceRow, b: DeviceRow, key: SortKey): number {
  switch (key) {
    case "state":
      return stateRank(a) - stateRank(b);
    case "name": {
      const left = rowName(a);
      const right = rowName(b);
      // A device with no name of any kind displays its address, and comparing
      // those as text puts 10.0.0.12 before 10.0.0.4. Fall back to numeric order
      // so a network of unnamed hosts still reads correctly.
      if (left === a.host.ip && right === b.host.ip) {
        return ipToNum(a.host.ip) - ipToNum(b.host.ip);
      }
      return left.localeCompare(right, undefined, { sensitivity: "base" });
    }
    case "ip":
      return ipToNum(a.host.ip) - ipToNum(b.host.ip);
    case "mac":
      return blankLast(a.host.mac, b.host.mac);
    case "vendor":
      return blankLast(a.host.vendor, b.host.vendor);
    case "os":
      return blankLast(a.host.os_guess, b.host.os_guess);
    case "ports":
      return a.host.open_ports.length - b.host.open_ports.length;
    case "response":
      // Missing measurements sort last in both directions rather than pretending
      // to be the fastest host on the network.
      return (a.host.response_ms ?? Number.MAX_SAFE_INTEGER) -
        (b.host.response_ms ?? Number.MAX_SAFE_INTEGER);
    case "last_seen":
      return a.host.last_seen.localeCompare(b.host.last_seen);
  }
}

/** Sort empty values after present ones, whichever direction is chosen. */
function blankLast(a: string | null, b: string | null): number {
  const left = a?.trim() ?? "";
  const right = b?.trim() ?? "";
  if (!left && !right) return 0;
  if (!left) return 1;
  if (!right) return -1;
  return left.localeCompare(right, undefined, { sensitivity: "base" });
}

/** Apply the filter and the sort in the order the table renders them. */
export function prepareRows(
  rows: DeviceRow[],
  filter: TableFilter,
  sortKey: SortKey,
  sortDir: SortDir,
): DeviceRow[] {
  return sortRows(filterRows(rows, filter), sortKey, sortDir);
}
