import { describe, expect, it } from "vitest";
import {
  EMPTY_DRAFT,
  draftKeyFor,
  notesFromDetail,
  reconcileDraft,
  type DraftRow,
  type DraftState,
} from "./drawerDraft";
import type { DeviceDetail } from "../types";

function row(overrides: Partial<DraftRow> & { ip?: string } = {}): DraftRow {
  const { ip, ...rest } = overrides;
  return {
    device_id: 7,
    custom_name: null,
    host: { ip: ip ?? "10.0.0.5" },
    ...rest,
  };
}

function detailFor(deviceId: number, notes: string | null): DeviceDetail {
  return {
    device: {
      id: deviceId,
      network_scope_id: 1,
      identity_key: `mac:AA:BB:CC:00:00:${String(deviceId).padStart(2, "0")}`,
      identity_source: "mac",
      mac: `AA:BB:CC:00:00:${String(deviceId).padStart(2, "0")}`,
      custom_name: null,
      hostname: "printer-01",
      vendor: "HP",
      last_ip: "10.0.0.5",
      first_seen: "2026-01-01T00:00:00Z",
      last_seen: "2026-07-01T00:00:00Z",
      status: "known",
      notes,
      observation_count: 3,
    },
    observations: [],
    previous_ips: [],
    recent_changes: [],
    events: [],
    network_name: "Home Wi-Fi",
    presence: "present",
  };
}

describe("draft keys", () => {
  it("keys a saved device by its persistent id, not its address", () => {
    expect(draftKeyFor(row({ device_id: 42, ip: "10.0.0.5" }), 1)).toBe("device:42");
    // Same device after a DHCP change: same key, so the draft follows it.
    expect(draftKeyFor(row({ device_id: 42, ip: "10.0.0.90" }), 1)).toBe("device:42");
  });

  it("keys an unsaved row by scan and address", () => {
    expect(draftKeyFor(row({ device_id: null, ip: "10.0.0.5" }), 3)).toBe("scan:3:ip:10.0.0.5");
    // A different scan's row at the same address is a different draft.
    expect(draftKeyFor(row({ device_id: null, ip: "10.0.0.5" }), 4)).toBe("scan:4:ip:10.0.0.5");
  });

  it("has no key without a row", () => {
    expect(draftKeyFor(null, 1)).toBeNull();
  });
});

describe("notes from the loaded detail", () => {
  it("returns the stored notes when the detail matches the row's device", () => {
    expect(notesFromDetail(row({ device_id: 7 }), detailFor(7, "rack 4"))).toBe("rack 4");
    expect(notesFromDetail(row({ device_id: 7 }), detailFor(7, null))).toBe("");
  });

  it("refuses a detail describing a different device", () => {
    // The detail loads asynchronously; during a quick selection change it can
    // briefly belong to the previously selected device.
    expect(notesFromDetail(row({ device_id: 7 }), detailFor(8, "wrong device"))).toBeNull();
    expect(notesFromDetail(row({ device_id: null }), detailFor(8, "wrong device"))).toBeNull();
    expect(notesFromDetail(row({ device_id: 7 }), null)).toBeNull();
  });
});

describe("draft reconciliation", () => {
  it("loads name and notes when the drawer moves to a device", () => {
    const state = reconcileDraft(
      EMPTY_DRAFT,
      row({ device_id: 7, custom_name: "Front printer" }),
      detailFor(7, "toner low"),
      1,
    );
    expect(state).toEqual({
      key: "device:7",
      name: "Front printer",
      notes: "toner low",
      nameDirty: false,
      notesDirty: false,
    });
  });

  it("keeps the draft when the same device changes IP", () => {
    const before = reconcileDraft(EMPTY_DRAFT, row({ device_id: 7, ip: "10.0.0.5" }), null, 1);
    const typing: DraftState = { ...before, notes: "half-typed", notesDirty: true };
    const after = reconcileDraft(typing, row({ device_id: 7, ip: "10.0.0.90" }), null, 1);
    expect(after.notes).toBe("half-typed");
    expect(after.notesDirty).toBe(true);
  });

  it("loads notes for a historical observation at an older IP", () => {
    // Viewing an old scan where the device still had 10.0.0.2: the key is the
    // device id, so the stored notes load exactly as for the current address.
    const state = reconcileDraft(
      EMPTY_DRAFT,
      row({ device_id: 7, ip: "10.0.0.2" }),
      detailFor(7, "still here"),
      99,
    );
    expect(state.notes).toBe("still here");
  });

  it("fills notes when the detail arrives after the drawer opened", () => {
    const opened = reconcileDraft(EMPTY_DRAFT, row({ device_id: 7 }), null, 1);
    expect(opened.notes).toBe("");
    const loaded = reconcileDraft(opened, row({ device_id: 7 }), detailFor(7, "rack 4"), 1);
    expect(loaded.notes).toBe("rack 4");
  });

  it("never overwrites typing with a late-arriving detail", () => {
    const opened = reconcileDraft(EMPTY_DRAFT, row({ device_id: 7 }), null, 1);
    const typing: DraftState = { ...opened, notes: "my edit", notesDirty: true };
    const loaded = reconcileDraft(typing, row({ device_id: 7 }), detailFor(7, "stored"), 1);
    expect(loaded.notes).toBe("my edit");
  });

  it("keeps typed input after a failed save", () => {
    // A failed save leaves the stored detail unchanged; the dirty draft must
    // survive every subsequent reconcile so nothing is silently discarded.
    const typing: DraftState = {
      key: "device:7",
      name: "",
      notes: "unsaved edit",
      nameDirty: false,
      notesDirty: true,
    };
    const after = reconcileDraft(typing, row({ device_id: 7 }), detailFor(7, "old stored"), 1);
    expect(after.notes).toBe("unsaved edit");
  });

  it("resets both fields when moving to a different device", () => {
    const typing: DraftState = {
      key: "device:7",
      name: "typed name",
      notes: "typed notes",
      nameDirty: true,
      notesDirty: true,
    };
    const moved = reconcileDraft(
      typing,
      row({ device_id: 8, custom_name: null }),
      detailFor(8, "other device"),
      1,
    );
    expect(moved.key).toBe("device:8");
    expect(moved.name).toBe("");
    expect(moved.notes).toBe("other device");
    expect(moved.nameDirty).toBe(false);
    expect(moved.notesDirty).toBe(false);
  });

  it("does not reset the notes draft when the device is renamed", () => {
    const before = reconcileDraft(
      EMPTY_DRAFT,
      row({ device_id: 7, custom_name: "Old name" }),
      detailFor(7, "keep me"),
      1,
    );
    const editing: DraftState = { ...before, notes: "editing…", notesDirty: true };
    // A rename patches the row's custom_name but the key does not change.
    const renamed = reconcileDraft(
      editing,
      row({ device_id: 7, custom_name: "New name" }),
      detailFor(7, "keep me"),
      1,
    );
    expect(renamed.notes).toBe("editing…");
    // The untouched name field follows the external rename.
    expect(renamed.name).toBe("New name");
  });

  it("keeps a dirty name field through incoming enrichment", () => {
    const before = reconcileDraft(EMPTY_DRAFT, row({ device_id: 7 }), null, 1);
    const editing: DraftState = { ...before, name: "typing…", nameDirty: true };
    // Enrichment updates host fields; the row identity is unchanged.
    const after = reconcileDraft(editing, row({ device_id: 7, ip: "10.0.0.5" }), null, 1);
    expect(after.name).toBe("typing…");
  });

  it("clears the draft when the drawer empties", () => {
    const filled = reconcileDraft(EMPTY_DRAFT, row(), null, 1);
    expect(reconcileDraft(filled, null, null, 1)).toEqual(EMPTY_DRAFT);
  });
});
