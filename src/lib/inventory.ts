// Inventory filtering, searching, sorting and column definitions.
//
// Pure, so the rules that decide what an operator sees are testable without
// rendering anything. The backend decides *facts* (presence, counts, addresses);
// everything here is presentation of those facts.

import type { InventoryRow, PresenceState } from "../types";
import { ipToNum, serviceLabel } from "./format";

export type InventoryColumn =
  | "device"
  | "address"
  | "network"
  | "manufacturer"
  | "status"
  | "services"
  | "last_seen"
  | "mac"
  | "hostname"
  | "first_seen"
  | "observations"
  | "response"
  | "previous";

export interface InventoryColumnDef {
  key: InventoryColumn;
  label: string;
  /** Hidden from the narrowest window up, lowest priority first. */
  priority: 1 | 2 | 3;
  align?: "right";
  /** Never hidden and never turned off: without these a row means nothing. */
  required?: boolean;
  /** Off unless the operator turns it on. */
  optional?: boolean;
}

/**
 * Column order, priority and defaults.
 *
 * Device and Address are the anchors and are always shown. Network is in the
 * default set but only rendered when there is more than one network to tell
 * apart, which is decided by the caller: a person with one network should not
 * be reminded of that on every row.
 */
export const INVENTORY_COLUMNS: InventoryColumnDef[] = [
  { key: "device", label: "Device", priority: 1, required: true },
  { key: "address", label: "Address", priority: 1, required: true },
  { key: "status", label: "Status", priority: 1 },
  { key: "services", label: "Services", priority: 2 },
  { key: "manufacturer", label: "Manufacturer", priority: 2 },
  { key: "network", label: "Network", priority: 2 },
  { key: "last_seen", label: "Last seen", priority: 1, align: "right" },
  { key: "mac", label: "MAC", priority: 3, optional: true },
  { key: "hostname", label: "Hostname", priority: 3, optional: true },
  { key: "first_seen", label: "First seen", priority: 3, optional: true, align: "right" },
  { key: "observations", label: "Scans", priority: 3, optional: true, align: "right" },
  { key: "response", label: "Response", priority: 3, optional: true, align: "right" },
  { key: "previous", label: "Previous address", priority: 3, optional: true },
];

export const OPTIONAL_INVENTORY_COLUMNS = INVENTORY_COLUMNS.filter((c) => c.optional);

/** Columns a given window width has room for, lowest priority dropping first. */
export function inventoryColumnsForWidth(width: number): InventoryColumn[] {
  const maxPriority = width < 900 ? 1 : width < 1100 ? 2 : 3;
  return INVENTORY_COLUMNS.filter((c) => c.required || c.priority <= maxPriority).map((c) => c.key);
}

/**
 * Which columns to render.
 *
 * What fits, intersected with what the operator turned on, minus the optional
 * columns they have not asked for, minus Network when there is only one. Device
 * and Address survive all of it.
 */
export function visibleInventoryColumns(
  width: number,
  enabledOptional: InventoryColumn[],
  multipleNetworks: boolean,
): InventoryColumn[] {
  const fits = new Set(inventoryColumnsForWidth(width));
  return INVENTORY_COLUMNS.filter((column) => {
    if (column.required) return true;
    if (!fits.has(column.key)) return false;
    if (column.key === "network") return multipleNetworks;
    if (column.optional) return enabledOptional.includes(column.key);
    return true;
  }).map((c) => c.key);
}

/** The filter views the header offers, in the order they appear. */
export type InventoryView =
  | "all"
  | "present"
  | "missing"
  | "unknown"
  | "trusted"
  | "unreviewed"
  | "ignored";

export const INVENTORY_VIEWS: Array<{ id: InventoryView; label: string; hint: string }> = [
  { id: "all", label: "All", hint: "Every device ArcScan has recorded" },
  {
    id: "present",
    label: "Present",
    hint: "Answered the latest completed scan of its network",
  },
  {
    id: "missing",
    label: "Missing",
    hint: "Seen before, absent from the latest completed scan",
  },
  {
    id: "unknown",
    label: "Unknown",
    hint: "No completed scan can say whether these answered",
  },
  { id: "trusted", label: "Trusted", hint: "Devices you marked trusted" },
  { id: "unreviewed", label: "Unreviewed", hint: "Devices you have not classified" },
  { id: "ignored", label: "Ignored", hint: "Devices whose changes you chose to hide" },
];

export interface InventoryFilter {
  query: string;
  view: InventoryView;
  /** Null means every network. */
  networkId: number | null;
}

export const EMPTY_INVENTORY_FILTER: InventoryFilter = {
  query: "",
  view: "all",
  networkId: null,
};

/**
 * Everything a search term is matched against, lowercased.
 *
 * Note text is included only as the excerpt the backend loads, because the table
 * carries an indicator rather than every note body. A search for a word deep in
 * a long note will not find it; that is a deliberate trade against loading
 * thousands of notes to draw a dot.
 */
export function inventoryHaystack(row: InventoryRow): string {
  return [
    row.display_name,
    row.custom_name ?? "",
    row.hostname ?? "",
    row.current_ip ?? "",
    row.previous_ips.join(" "),
    row.mac ?? "",
    row.vendor ?? "",
    row.os_guess ?? "",
    row.network_name ?? "",
    row.status,
    row.presence,
    row.open_ports.join(" "),
    row.open_ports.map(serviceLabel).join(" "),
    row.notes_excerpt ?? "",
  ]
    .join(" ")
    .toLowerCase();
}

/**
 * True when a row belongs in a view.
 *
 * Observed presence and operator classification are kept apart on purpose: a
 * device can be Trusted *and* Missing, and collapsing the two would make one of
 * those facts unreachable.
 */
export function matchesView(row: InventoryRow, view: InventoryView): boolean {
  switch (view) {
    case "all":
      return true;
    case "present":
    case "missing":
    case "unknown":
      return row.presence === (view as PresenceState);
    case "trusted":
      return row.status === "trusted";
    case "unreviewed":
      return row.status === "unclassified";
    case "ignored":
      return row.status === "ignored";
  }
}

export function filterInventory(rows: InventoryRow[], filter: InventoryFilter): InventoryRow[] {
  const query = filter.query.trim().toLowerCase();
  // Every term must match, so "printer 443" narrows rather than widens.
  const terms = query ? query.split(/\s+/) : [];
  return rows.filter((row) => {
    if (filter.networkId != null && row.network_scope_id !== filter.networkId) return false;
    if (!matchesView(row, filter.view)) return false;
    if (terms.length === 0) return true;
    const hay = inventoryHaystack(row);
    return terms.every((term) => hay.includes(term));
  });
}

export type InventorySortKey = InventoryColumn;
export type SortDirection = "asc" | "desc";

/** Attention-worthy states sort first: missing, then unknown, then present. */
function presenceRank(row: InventoryRow): number {
  switch (row.presence) {
    case "missing":
      return 0;
    case "unknown":
      return 1;
    default:
      return 2;
  }
}

function blankLast(a: string | null, b: string | null): number {
  const left = a?.trim() ?? "";
  const right = b?.trim() ?? "";
  if (!left && !right) return 0;
  if (!left) return 1;
  if (!right) return -1;
  return left.localeCompare(right, undefined, { sensitivity: "base" });
}

function compareBy(a: InventoryRow, b: InventoryRow, key: InventorySortKey): number {
  switch (key) {
    case "device":
      return a.display_name.localeCompare(b.display_name, undefined, { sensitivity: "base" });
    case "address":
      return ipToNum(a.current_ip ?? "") - ipToNum(b.current_ip ?? "");
    case "network":
      return blankLast(a.network_name, b.network_name);
    case "manufacturer":
      return blankLast(a.vendor, b.vendor);
    case "status":
      return presenceRank(a) - presenceRank(b);
    case "services":
      return a.open_ports.length - b.open_ports.length;
    case "last_seen":
      return a.last_seen.localeCompare(b.last_seen);
    case "first_seen":
      return a.first_seen.localeCompare(b.first_seen);
    case "mac":
      return blankLast(a.mac, b.mac);
    case "hostname":
      return blankLast(a.hostname, b.hostname);
    case "observations":
      return a.observation_count - b.observation_count;
    case "response":
      // A device with no measurement sorts last in both directions rather than
      // pretending to be the fastest thing on the network.
      return (
        (a.latest_response_ms ?? Number.MAX_SAFE_INTEGER) -
        (b.latest_response_ms ?? Number.MAX_SAFE_INTEGER)
      );
    case "previous":
      return blankLast(a.previous_ips[0] ?? null, b.previous_ips[0] ?? null);
  }
}

export function sortInventory(
  rows: InventoryRow[],
  key: InventorySortKey,
  dir: SortDirection,
): InventoryRow[] {
  const factor = dir === "asc" ? 1 : -1;
  return rows.slice().sort((a, b) => {
    const primary = compareBy(a, b, key);
    if (primary !== 0) return primary * factor;
    // Address is the tiebreak everywhere, so equal values never shuffle between
    // renders. Device id breaks the last tie so the order is total.
    const byIp = ipToNum(a.current_ip ?? "") - ipToNum(b.current_ip ?? "");
    if (byIp !== 0) return byIp * (key === "address" ? factor : 1);
    return a.device_id - b.device_id;
  });
}

export function prepareInventory(
  rows: InventoryRow[],
  filter: InventoryFilter,
  key: InventorySortKey,
  dir: SortDirection,
): InventoryRow[] {
  return sortInventory(filterInventory(rows, filter), key, dir);
}

/** The compact header summary, e.g. `86 devices · 62 present · 9 missing`. */
export function inventoryHeadline(counts: {
  total: number;
  present: number;
  missing: number;
  unknown: number;
}): string {
  const parts = [`${counts.total.toLocaleString()} ${counts.total === 1 ? "device" : "devices"}`];
  if (counts.present > 0) parts.push(`${counts.present.toLocaleString()} present`);
  if (counts.missing > 0) parts.push(`${counts.missing.toLocaleString()} missing`);
  if (counts.unknown > 0) parts.push(`${counts.unknown.toLocaleString()} unknown`);
  return parts.join(" · ");
}

export const PRESENCE_LABEL: Record<PresenceState, string> = {
  present: "Present",
  missing: "Missing",
  unknown: "Unknown",
};

/** The full sentence behind each presence word, used as a tooltip and in help. */
export const PRESENCE_HINT: Record<PresenceState, string> = {
  present: "Answered the latest completed scan of this network.",
  missing: "Seen before by a scan with the same coverage, absent from the latest completed one.",
  unknown:
    "No completed scan with matching coverage has run for this network, so ArcScan cannot say.",
};
