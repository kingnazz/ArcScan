import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import { useUpdater } from "./useUpdater";

/**
 * The frontend half of the portable updater gate.
 *
 * The backend half is stronger and is what the release actually rests on: a
 * portable build does not link tauri-plugin-updater, so there is no
 * install-and-relaunch path in the binary. These cover the layer above it, so
 * the two agree -- a portable interface that offered an install action would be
 * wrong even if pressing it did nothing.
 */
const downloadAndInstall = vi.fn(async () => {});
const relaunch = vi.fn(async () => {});

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: async () => ({ version: "1.8.5", body: "notes", downloadAndInstall }),
}));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch }));

beforeEach(() => {
  downloadAndInstall.mockClear();
  relaunch.mockClear();
  // useUpdater no-ops outside Tauri, so the tests have to look like it.
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("useUpdater in installer mode", () => {
  it("finds an update and installs it, exactly as before 1.8.4", async () => {
    const { result } = renderHook(() => useUpdater(true, "installer"));
    await waitFor(() => expect(result.current.status).toBe("available"));
    expect(result.current.version).toBe("1.8.5");

    await act(async () => {
      await result.current.install();
    });

    expect(downloadAndInstall).toHaveBeenCalledTimes(1);
    expect(relaunch).toHaveBeenCalledTimes(1);
  });
});

describe("useUpdater in manual mode", () => {
  it("still reports the newer version, so the interface can say so", async () => {
    const { result } = renderHook(() => useUpdater(true, "manual"));
    await waitFor(() => expect(result.current.status).toBe("available"));
    expect(result.current.version).toBe("1.8.5");
    expect(result.current.mode).toBe("manual");
  });

  it("never downloads, never installs and never relaunches", async () => {
    const { result } = renderHook(() => useUpdater(true, "manual"));
    await waitFor(() => expect(result.current.status).toBe("available"));

    await act(async () => {
      await result.current.install();
    });

    expect(downloadAndInstall).not.toHaveBeenCalled();
    expect(relaunch).not.toHaveBeenCalled();
    // And it does not pretend to be doing something either.
    expect(result.current.status).toBe("available");
  });
});
