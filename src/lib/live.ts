// Live result merging.
//
// The scanner streams hosts as it finds them and then again once names, MACs and
// vendors resolve, so the table is built up incrementally rather than appearing
// all at once when the scan ends. These functions are pure and keyed by address,
// which is what guarantees a second event for the same host updates its row
// instead of adding a duplicate.

import type {
  ChangeKind,
  DeviceStatus,
  FieldChange,
  HostResult,
  ScanComparison,
  ScanDetail,
} from "../types";
import { ipToNum } from "./format";

/** One row of the results table: an observation plus what we know about it. */
export interface DeviceRow {
  host: HostResult;
  device_id: number | null;
  /** Operator-supplied name, which wins over the hostname for display. */
  custom_name: string | null;
  status: DeviceStatus;
  first_seen: string | null;
  /** How this device differs from the baseline scan, once one is known. */
  change: ChangeKind | null;
  /** Field-level differences, for the changed indicator's tooltip. */
  changed_fields: FieldChange[];
  /** True while the scan is still enriching the row. */
  pending: boolean;
}

function blankRow(host: HostResult, pending: boolean): DeviceRow {
  return {
    host,
    device_id: null,
    custom_name: null,
    status: "unclassified",
    first_seen: null,
    change: null,
    changed_fields: [],
    pending,
  };
}

/**
 * Insert or update a host, keeping the list in ascending address order.
 *
 * Address order is the default the operator expects while a scan streams, and
 * inserting in place means a row never jumps position as later events arrive.
 * The table applies the operator's chosen sort on top of this.
 */
export function upsertHost(rows: DeviceRow[], host: HostResult, pending: boolean): DeviceRow[] {
  const index = rows.findIndex((r) => r.host.ip === host.ip);
  if (index >= 0) {
    const existing = rows[index];
    const next = rows.slice();
    next[index] = {
      ...existing,
      // Merge rather than replace: an update that could not resolve a hostname
      // must not erase one an earlier event already reported.
      host: mergeHost(existing.host, host),
      pending,
    };
    return next;
  }
  const row = blankRow(host, pending);
  const target = ipToNum(host.ip);
  let insertAt = rows.length;
  for (let i = 0; i < rows.length; i++) {
    if (ipToNum(rows[i].host.ip) > target) {
      insertAt = i;
      break;
    }
  }
  const next = rows.slice();
  next.splice(insertAt, 0, row);
  return next;
}

/** Later information wins, but a null never overwrites a value we already have. */
export function mergeHost(before: HostResult, after: HostResult): HostResult {
  return {
    ip: after.ip,
    hostname: after.hostname ?? before.hostname,
    mac: after.mac ?? before.mac,
    vendor: after.vendor ?? before.vendor,
    // Ports come from the probe pass and are complete the first time.
    open_ports: after.open_ports.length > 0 ? after.open_ports : before.open_ports,
    response_ms: after.response_ms ?? before.response_ms,
    icmp_ms: after.icmp_ms ?? before.icmp_ms,
    tcp_ms: after.tcp_ms ?? before.tcp_ms,
    ttl: after.ttl ?? before.ttl,
    os_guess: after.os_guess ?? before.os_guess,
    last_seen: after.last_seen || before.last_seen,
  };
}

/**
 * Withdraw a host the scanner reported during probing but ruled out afterwards.
 *
 * On a local segment a real device has to answer ARP, and a transparent router
 * can accept TCP for addresses where nothing exists. Removing those rows is what
 * keeps the streamed table identical to what gets saved.
 */
export function removeHostByIp(rows: DeviceRow[], ip: string): DeviceRow[] {
  return rows.filter((r) => r.host.ip !== ip);
}

/** Mark every row as fully enriched, once a scan has finished. */
export function settleRows(rows: DeviceRow[]): DeviceRow[] {
  return rows.map((row) => (row.pending ? { ...row, pending: false } : row));
}

/**
 * Fold a comparison into the rows so the table can mark new, returned and
 * changed devices.
 */
export function applyComparison(rows: DeviceRow[], comparison: ScanComparison | null): DeviceRow[] {
  if (!comparison) return rows;
  const byIp = new Map<string, { kind: ChangeKind; fields: FieldChange[] }>();
  for (const diff of comparison.added) {
    byIp.set(diff.ip, { kind: diff.kind, fields: diff.fields });
  }
  for (const diff of comparison.changed) {
    byIp.set(diff.ip, { kind: diff.kind, fields: diff.fields });
  }
  return rows.map((row) => {
    const hit = byIp.get(row.host.ip);
    return hit
      ? { ...row, change: hit.kind, changed_fields: hit.fields }
      : { ...row, change: null, changed_fields: [] };
  });
}

/** Attach device identities from a save, so names appear without a reload. */
export function applyDeviceIdentities(
  rows: DeviceRow[],
  devices: Array<{
    ip: string;
    device_id: number | null;
    custom_name: string | null;
    status: DeviceStatus;
    first_seen: string | null;
  }>,
): DeviceRow[] {
  const byIp = new Map(devices.map((d) => [d.ip, d]));
  return rows.map((row) => {
    const device = byIp.get(row.host.ip);
    if (!device) return row;
    return {
      ...row,
      device_id: device.device_id,
      custom_name: device.custom_name,
      status: device.status,
      first_seen: device.first_seen,
    };
  });
}

/** Build rows for a scan reopened from history. */
export function rowsFromScanDetail(detail: ScanDetail): DeviceRow[] {
  const byIp = new Map(detail.devices.map((d) => [d.ip, d]));
  return detail.hosts
    .map((host) => {
      const device = byIp.get(host.ip);
      return {
        ...blankRow(host, false),
        device_id: device?.device_id ?? null,
        custom_name: device?.custom_name ?? null,
        status: device?.status ?? ("unclassified" as DeviceStatus),
        first_seen: device?.first_seen ?? null,
      };
    })
    .sort((a, b) => ipToNum(a.host.ip) - ipToNum(b.host.ip));
}

/** The name to show for a row, matching the backend's `display_name` order. */
export function rowName(row: DeviceRow): string {
  const pick = (s: string | null | undefined) => {
    const trimmed = s?.trim();
    return trimmed ? trimmed : null;
  };
  return (
    pick(row.custom_name) ??
    pick(row.host.hostname) ??
    (pick(row.host.vendor) ? `${row.host.vendor?.trim()} (${row.host.ip})` : null) ??
    row.host.ip
  );
}

/** True when a row carries no name of its own and only the address identifies it. */
export function isUnnamed(row: DeviceRow): boolean {
  return !row.custom_name?.trim() && !row.host.hostname?.trim();
}

/**
 * Track which scan the UI is showing so events from an older one are ignored.
 *
 * A cancelled scan keeps streaming a few events while its tasks wind down. If
 * those landed in the next scan's table they would show devices that are not on
 * the network the operator just asked about, which is worse than showing nothing.
 */
export function isStaleEvent(eventScanId: number, activeScanId: number | null): boolean {
  if (activeScanId == null) return true;
  return eventScanId !== activeScanId;
}
