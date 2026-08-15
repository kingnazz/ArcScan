// Changes-inbox filtering, grouping and copy.
//
// Pure, so what an operator sees in the inbox — and, more importantly, what they
// do not see — is testable without rendering anything.

import type { ChangeEvent, ChangeType } from "../types";
import { serviceWithPort } from "./format";
import { deviceTypeLabel, serviceName } from "./discovery";

/** The filters the inbox header offers, in the order they appear. */
export type ChangeView =
  | "unreviewed"
  | "all"
  | "added"
  | "missing"
  | "returned"
  | "address"
  | "name"
  | "services"
  | "acknowledged"
  | "ignored";

export const CHANGE_VIEWS: Array<{ id: ChangeView; label: string }> = [
  { id: "unreviewed", label: "Unreviewed" },
  { id: "all", label: "All changes" },
  { id: "added", label: "New devices" },
  { id: "missing", label: "Missing devices" },
  { id: "returned", label: "Returned devices" },
  { id: "address", label: "Address changes" },
  { id: "name", label: "Name changes" },
  { id: "services", label: "Service changes" },
  { id: "acknowledged", label: "Acknowledged" },
  { id: "ignored", label: "Ignored" },
];

/** How far back the inbox is looking. */
export type ChangeWindow = "all" | "7d" | "30d";

export const CHANGE_WINDOWS: Array<{ id: ChangeWindow; label: string; days: number | null }> = [
  { id: "all", label: "Any time", days: null },
  { id: "7d", label: "Last 7 days", days: 7 },
  { id: "30d", label: "Last 30 days", days: 30 },
];

export interface ChangeFilter {
  query: string;
  view: ChangeView;
  window: ChangeWindow;
  /** Null means every network. */
  networkId: number | null;
}

export const EMPTY_CHANGE_FILTER: ChangeFilter = {
  view: "unreviewed",
  query: "",
  window: "all",
  networkId: null,
};

const NAME_TYPES: ChangeType[] = ["hostname_changed", "vendor_changed", "os_changed"];

/**
 * True when an event belongs in a view.
 *
 * Ignored events are excluded from every view except Ignored itself, including
 * All: "All changes" means everything still in the inbox, and an operator who
 * ignored a device has said they do not want it there. The Ignored filter brings
 * them back, and nothing is ever deleted.
 */
export function matchesChangeView(event: ChangeEvent, view: ChangeView): boolean {
  if (view === "ignored") return event.state === "ignored";
  if (event.state === "ignored") return false;
  switch (view) {
    case "all":
      return true;
    case "unreviewed":
      return event.state === "unreviewed";
    case "acknowledged":
      return event.state === "acknowledged";
    case "added":
      return event.change_type === "device_added";
    case "missing":
      return event.change_type === "device_missing";
    case "returned":
      return event.change_type === "device_returned";
    case "address":
      return event.change_type === "ip_changed" || event.change_type === "mac_changed";
    case "name":
      return NAME_TYPES.includes(event.change_type);
    case "services":
      return event.change_type === "ports_changed";
  }
}

/** When the change was found, falling back to when it was recorded. */
export function changeTimestamp(event: ChangeEvent): string {
  return event.scan_at ?? event.created_at;
}

export function changeHaystack(event: ChangeEvent): string {
  return [
    event.device_label,
    event.ip ?? "",
    event.mac ?? "",
    event.vendor ?? "",
    event.network_name ?? "",
    event.change_type.replace(/_/g, " "),
    event.old_value ?? "",
    event.new_value ?? "",
    event.opened_ports.map(serviceWithPort).join(" "),
    event.closed_ports.map(serviceWithPort).join(" "),
  ]
    .join(" ")
    .toLowerCase();
}

export function filterChanges(
  events: ChangeEvent[],
  filter: ChangeFilter,
  now: number = Date.now(),
): ChangeEvent[] {
  const days = CHANGE_WINDOWS.find((w) => w.id === filter.window)?.days ?? null;
  const cutoff = days == null ? null : now - days * 24 * 3600 * 1000;
  const terms = filter.query.trim().toLowerCase()
    ? filter.query.trim().toLowerCase().split(/\s+/)
    : [];

  return events.filter((event) => {
    if (filter.networkId != null && event.network_scope_id !== filter.networkId) return false;
    if (!matchesChangeView(event, filter.view)) return false;
    if (cutoff != null) {
      const at = new Date(changeTimestamp(event)).getTime();
      // An unparseable date is kept rather than silently dropped: hiding a real
      // change because its timestamp is odd is the worse failure.
      if (Number.isFinite(at) && at < cutoff) return false;
    }
    if (terms.length === 0) return true;
    const hay = changeHaystack(event);
    return terms.every((term) => hay.includes(term));
  });
}

/** One device's changes from one scan, shown together. */
export interface ChangeGroup {
  key: string;
  deviceId: number | null;
  deviceLabel: string;
  networkName: string | null;
  ip: string | null;
  scanId: number | null;
  baselineScanId: number | null;
  at: string;
  events: ChangeEvent[];
}

/**
 * Group related changes for one device in one scan.
 *
 * A device that moved address *and* opened a port is one thing that happened,
 * not two, and reading it as two makes the inbox feel longer than the network
 * actually is. The underlying events stay separate and individually reviewable.
 */
export function groupChanges(events: ChangeEvent[]): ChangeGroup[] {
  const groups = new Map<string, ChangeGroup>();
  for (const event of events) {
    const subject = event.device_id != null ? `d${event.device_id}` : `ip:${event.ip ?? "?"}`;
    const key = `s${event.scan_id ?? "none"}|${subject}`;
    const existing = groups.get(key);
    if (existing) {
      existing.events.push(event);
      continue;
    }
    groups.set(key, {
      key,
      deviceId: event.device_id,
      deviceLabel: event.device_label,
      networkName: event.network_name,
      ip: event.ip,
      scanId: event.scan_id,
      baselineScanId: event.baseline_scan_id,
      at: changeTimestamp(event),
      events: [event],
    });
  }
  return [...groups.values()];
}

/** The single line that describes what happened, e.g. `Opened: HTTPS · 443`. */
export function describeChange(event: ChangeEvent): string {
  switch (event.change_type) {
    case "device_added":
      return "First seen in this scan";
    case "device_returned":
      return "Seen again after being absent";
    case "device_missing":
      return "Did not answer this scan";
    case "ports_changed": {
      const parts: string[] = [];
      if (event.opened_ports.length > 0) {
        parts.push(`Opened: ${event.opened_ports.map(serviceWithPort).join(", ")}`);
      }
      if (event.closed_ports.length > 0) {
        parts.push(`Closed: ${event.closed_ports.map(serviceWithPort).join(", ")}`);
      }
      return parts.join(" · ") || "Open services changed";
    }
    case "service_appeared":
      return `Now advertising ${serviceName(event.new_value ?? "a service")}`;
    case "service_disappeared":
      // The wording is careful: ArcScan knows the device stopped *advertising*
      // it, which is not the same as the service being switched off.
      return `No longer advertising ${serviceName(event.old_value ?? "a service")}`;
    case "device_type_changed":
      return `${deviceTypeLabel(event.old_value)} → ${deviceTypeLabel(event.new_value)}`;
    default:
      return `${event.old_value ?? "none"} → ${event.new_value ?? "none"}`;
  }
}

/**
 * Which actions an event offers.
 *
 * Trust only appears where it would do something: on a device that is not
 * already trusted. Acknowledge disappears once the event is acknowledged, and
 * Ignore disappears once the device is ignored, so no button is ever a no-op.
 */
export type ChangeAction = "review" | "trust" | "rename" | "ignore" | "acknowledge" | "reopen";

export function actionsFor(event: ChangeEvent): ChangeAction[] {
  const actions: ChangeAction[] = [];
  if (event.device_id != null) actions.push("review");
  if (event.change_type === "device_added" && event.device_id != null) {
    if (event.device_status !== "trusted") actions.push("trust");
    actions.push("rename");
  }
  if (event.state === "acknowledged" || event.state === "ignored") {
    actions.push("reopen");
  } else {
    actions.push("acknowledge");
  }
  if (event.device_id != null && event.device_status !== "ignored" && event.state !== "ignored") {
    actions.push("ignore");
  }
  return actions;
}
