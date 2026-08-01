// Draft identity for the device drawer's name and notes fields.
//
// Drafts must follow the *device*, not the address: a device can change IP
// between scans while remaining the same persistent device, and a historical
// observation can show an older IP than the device holds today. Keying drafts
// by IP (as v1.7.0 did) made notes fail to load after an address change and
// let half-typed text follow the wrong device. The rules live here, pure and
// separate from the component, so they can be tested directly.

import type { DeviceDetail } from "../types";

/** The slice of a table row the draft logic needs. */
export interface DraftRow {
  device_id: number | null;
  custom_name: string | null;
  host: { ip: string };
}

/**
 * The identity a draft belongs to.
 *
 * A saved device is keyed by its persistent id, so the draft survives IP
 * changes, renames and incoming enrichment. A row not yet saved has no device
 * id; it is keyed by scan and address, which is the only stable identity it
 * has, and scoping it to the scan stops a later scan's same-address row from
 * inheriting the draft.
 */
export function draftKeyFor(
  row: DraftRow | null,
  scanKey: number | string | null,
): string | null {
  if (!row) return null;
  if (row.device_id != null) return `device:${row.device_id}`;
  return `scan:${scanKey ?? "live"}:ip:${row.host.ip}`;
}

export interface DraftState {
  key: string | null;
  name: string;
  notes: string;
  /** True once the operator has typed; a dirty draft is never overwritten. */
  nameDirty: boolean;
  notesDirty: boolean;
}

export const EMPTY_DRAFT: DraftState = {
  key: null,
  name: "",
  notes: "",
  nameDirty: false,
  notesDirty: false,
};

/**
 * The notes the store holds for this row, or null when the loaded detail does
 * not belong to this row's device. The detail is fetched asynchronously, so
 * during a quick selection change it can briefly describe the previously
 * selected device — those notes must never leak into this draft.
 */
export function notesFromDetail(row: DraftRow, detail: DeviceDetail | null): string | null {
  if (!detail) return null;
  if (row.device_id == null || detail.device.id !== row.device_id) return null;
  return detail.device.notes ?? "";
}

/**
 * Reconcile the draft with the current row and detail.
 *
 * Moving to a different draft key resets both fields from stored values.
 * Staying on the same key only fills fields the operator has not touched:
 * a detail that arrives after opening fills the notes, an external rename
 * (e.g. an undo) refreshes an untouched name field, and incoming scan
 * enrichment changes neither, because the key does not change.
 */
export function reconcileDraft(
  current: DraftState,
  row: DraftRow | null,
  detail: DeviceDetail | null,
  scanKey: number | string | null,
): DraftState {
  const key = draftKeyFor(row, scanKey);
  if (row == null || key == null) {
    return current.key == null ? current : EMPTY_DRAFT;
  }

  if (key !== current.key) {
    return {
      key,
      name: row.custom_name ?? "",
      notes: notesFromDetail(row, detail) ?? "",
      nameDirty: false,
      notesDirty: false,
    };
  }

  let next = current;
  if (!current.nameDirty) {
    const name = row.custom_name ?? "";
    if (name !== current.name) next = { ...next, name };
  }
  if (!current.notesDirty) {
    const notes = notesFromDetail(row, detail);
    if (notes != null && notes !== current.notes) next = { ...next, notes };
  }
  return next;
}
