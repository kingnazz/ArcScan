import { describe, expect, it, vi } from "vitest";
import { createCredentialStore, type CredentialOperations } from "./windowsCredential";
import type { WindowsCredentialStatus } from "../types";

const UNSET: WindowsCredentialStatus = {
  configured: false,
  account: null,
  supported: true,
  unsupported_reason: null,
};

const SET: WindowsCredentialStatus = {
  configured: true,
  account: "CORP\\admin",
  supported: true,
  unsupported_reason: null,
};

/** The most recent value a listener saw. */
function last<T>(values: T[]): T | undefined {
  return values[values.length - 1];
}

/** A backend whose stored credential the test can change. */
function fakeBackend(initial: WindowsCredentialStatus = UNSET) {
  let stored = initial;
  const operations: CredentialOperations = {
    status: vi.fn(async () => stored),
    set: vi.fn(async (username: string) => {
      stored = { ...SET, account: username };
      return stored;
    }),
    clear: vi.fn(async () => {
      stored = UNSET;
      return stored;
    }),
  };
  return { operations, peek: () => stored };
}

describe("windows credential store", () => {
  it("tells subscribers when a credential is set", async () => {
    // The v1.9.0 bug, first direction: the command bar kept saying no
    // credential was set after Settings added one.
    const { operations } = fakeBackend();
    const store = createCredentialStore(operations);
    const seen: Array<boolean | null> = [];
    store.subscribe((status) => seen.push(status?.configured ?? null));

    await store.refresh();
    expect(store.current()?.configured).toBe(false);

    await store.set("CORP\\admin", null, "pw");
    expect(store.current()?.configured).toBe(true);
    expect(seen).toContain(true);
  });

  it("tells subscribers when a credential is forgotten", async () => {
    // The other direction: the picker kept showing a credential after Forget.
    const { operations } = fakeBackend(SET);
    const store = createCredentialStore(operations);
    await store.refresh();
    const seen: Array<boolean> = [];
    store.subscribe((status) => seen.push(status?.configured === true));

    await store.clear();
    expect(store.current()?.configured).toBe(false);
    expect(last(seen)).toBe(false);
  });

  it("gives a late subscriber the status immediately", async () => {
    // A component that mounts after the first read must not wait for the next
    // change to show anything.
    const { operations } = fakeBackend(SET);
    const store = createCredentialStore(operations);
    await store.refresh();

    const seen: Array<string | null> = [];
    store.subscribe((status) => seen.push(status?.account ?? null));
    expect(seen).toEqual(["CORP\\admin"]);
  });

  it("does not notify a subscriber that has unsubscribed", async () => {
    const { operations } = fakeBackend();
    const store = createCredentialStore(operations);
    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);
    unsubscribe();

    await store.set("admin", null, "pw");
    expect(listener).not.toHaveBeenCalled();
  });

  it("keeps notifying the others when one subscriber throws", async () => {
    // One bad component must not freeze every other reader's view.
    const { operations } = fakeBackend();
    const store = createCredentialStore(operations);
    store.subscribe(() => {
      throw new Error("bad component");
    });
    const healthy = vi.fn();
    store.subscribe(healthy);

    await store.set("admin", null, "pw");
    expect(healthy).toHaveBeenCalled();
  });

  it("survives a backend that cannot answer", async () => {
    // A build with no credential support must not take out the component that
    // asked. The unset reading is the safe one for the picker's warning.
    const operations: CredentialOperations = {
      status: vi.fn(async () => {
        throw new Error("no IPC here");
      }),
      set: vi.fn(),
      clear: vi.fn(),
    };
    const store = createCredentialStore(operations);
    await expect(store.refresh()).resolves.toBeNull();
    expect(store.current()).toBeNull();
  });

  it("reflects a change made through the store from anywhere", async () => {
    // Two readers, one writer: the shape Settings and the command bar have.
    const { operations } = fakeBackend();
    const store = createCredentialStore(operations);
    const picker: Array<boolean> = [];
    const settings: Array<boolean> = [];
    store.subscribe((s) => picker.push(s?.configured === true));
    store.subscribe((s) => settings.push(s?.configured === true));

    await store.set("CORP\\admin", null, "pw");
    expect(last(picker)).toBe(true);
    expect(last(settings)).toBe(true);

    await store.clear();
    expect(last(picker)).toBe(false);
    expect(last(settings)).toBe(false);
  });

  it("never holds a password", () => {
    // There is no command that reads one back, so there is nothing to cache.
    // This asserts the surface has no field for one.
    const { operations } = fakeBackend();
    const store = createCredentialStore(operations);
    expect(Object.keys(store).sort()).toEqual([
      "clear",
      "current",
      "refresh",
      "set",
      "subscribe",
    ]);
    expect(JSON.stringify(store.current() ?? {})).not.toContain("password");
  });
});
