import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { usePublicIp } from "./usePublicIp";
import { api } from "../lib/api";
import { PublicIpError, abortError } from "../lib/publicIp";

const IP = "203.0.113.24";
const OTHER_IP = "198.51.100.17";

/** A promise whose settlement this test controls. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

let lookup: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  lookup = vi.spyOn(api, "publicIp");
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("usePublicIp", () => {
  it("starts not checked and looks nothing up on mount", async () => {
    const { result } = renderHook(() => usePublicIp());

    expect(result.current.state).toEqual({ status: "idle" });
    // The privacy guarantee of the whole feature: mounting the hook, which the
    // app does on launch, must not contact anyone.
    expect(lookup).not.toHaveBeenCalled();

    // Re-rendering, which a view switch or a finished scan causes, must not
    // start one either.
    await act(async () => {});
    expect(lookup).not.toHaveBeenCalled();
  });

  it("goes through checking to the address once Check is pressed", async () => {
    const gate = deferred<string>();
    lookup.mockReturnValue(gate.promise);
    const { result } = renderHook(() => usePublicIp());

    act(() => {
      void result.current.check();
    });
    expect(result.current.state.status).toBe("checking");

    await act(async () => {
      gate.resolve(IP);
      await gate.promise;
    });

    expect(result.current.state).toMatchObject({ status: "ready", ip: IP });
    // The freshness indicator needs a time to count from.
    expect((result.current.state as { checkedAt: number }).checkedAt).toBeGreaterThan(0);
    expect(lookup).toHaveBeenCalledTimes(1);
  });

  it("ignores repeated presses while a lookup is already running", async () => {
    const gate = deferred<string>();
    lookup.mockReturnValue(gate.promise);
    const { result } = renderHook(() => usePublicIp());

    act(() => {
      void result.current.check();
      void result.current.check();
      void result.current.check();
    });

    // Three presses, one outbound request.
    expect(lookup).toHaveBeenCalledTimes(1);

    await act(async () => {
      gate.resolve(IP);
      await gate.promise;
    });
    expect(result.current.state).toMatchObject({ status: "ready", ip: IP });
  });

  it("allows a refresh once the previous lookup has finished", async () => {
    lookup.mockResolvedValueOnce(IP).mockResolvedValueOnce(OTHER_IP);
    const { result } = renderHook(() => usePublicIp());

    await act(async () => {
      await result.current.check();
    });
    expect(result.current.state).toMatchObject({ status: "ready", ip: IP });

    await act(async () => {
      await result.current.check();
    });
    expect(result.current.state).toMatchObject({ status: "ready", ip: OTHER_IP });
    expect(lookup).toHaveBeenCalledTimes(2);
  });

  it("drops a response that arrives after the value was forgotten", async () => {
    const gate = deferred<string>();
    lookup.mockReturnValue(gate.promise);
    const { result } = renderHook(() => usePublicIp());

    act(() => {
      void result.current.check();
    });
    act(() => {
      result.current.clear();
    });
    expect(result.current.state).toEqual({ status: "idle" });

    // The in-flight request answers anyway. It is stale, so it must not put an
    // address back on screen after the operator asked to forget it.
    await act(async () => {
      gate.resolve(IP);
      await gate.promise;
    });
    expect(result.current.state).toEqual({ status: "idle" });
  });

  it("drops a stale failure too", async () => {
    const gate = deferred<string>();
    lookup.mockReturnValue(gate.promise);
    const { result } = renderHook(() => usePublicIp());

    act(() => {
      void result.current.check();
    });
    act(() => {
      result.current.clear();
    });

    await act(async () => {
      gate.reject(new PublicIpError("No public-IP provider answered.", "ipify: HTTP 500"));
      await gate.promise.catch(() => {});
    });
    expect(result.current.state).toEqual({ status: "idle" });
  });

  it("reports a failure with its technical detail, and recovers on retry", async () => {
    lookup.mockRejectedValueOnce(
      new PublicIpError("No public-IP provider answered.", "ipify: HTTP 500"),
    );
    const { result } = renderHook(() => usePublicIp());

    await act(async () => {
      await result.current.check();
    });
    expect(result.current.state).toEqual({
      status: "error",
      message: "No public-IP provider answered.",
      technical: "ipify: HTTP 500",
    });

    // A failed lookup must not leave the control wedged: Retry starts a new one.
    lookup.mockResolvedValueOnce(IP);
    await act(async () => {
      await result.current.check();
    });
    expect(result.current.state).toMatchObject({ status: "ready", ip: IP });
  });

  it("stays quiet when a lookup is cancelled rather than failing", async () => {
    lookup.mockRejectedValueOnce(abortError());
    const { result } = renderHook(() => usePublicIp());

    await act(async () => {
      await result.current.check();
    });
    // A cancellation is not a provider fault and must not be reported as one.
    expect(result.current.state).toEqual({ status: "idle" });
  });

  it("gives up on a provider that never answers", async () => {
    vi.useFakeTimers();
    // A provider that hangs: it only settles when the hook's timeout aborts it.
    lookup.mockImplementation(
      (signal?: AbortSignal) =>
        new Promise<string>((_resolve, reject) => {
          signal?.addEventListener("abort", () => reject(abortError()), { once: true });
        }),
    );
    const { result } = renderHook(() => usePublicIp());

    act(() => {
      void result.current.check();
    });
    expect(result.current.state.status).toBe("checking");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(8_000);
    });

    expect(result.current.state).toMatchObject({
      status: "error",
      message: "The lookup timed out.",
    });
  });

  it("forgets a value it already has", async () => {
    lookup.mockResolvedValueOnce(IP);
    const { result } = renderHook(() => usePublicIp());

    await act(async () => {
      await result.current.check();
    });
    expect(result.current.state).toMatchObject({ status: "ready", ip: IP });

    act(() => {
      result.current.clear();
    });
    expect(result.current.state).toEqual({ status: "idle" });
  });

  it("writes nothing to storage, so the address dies with the session", async () => {
    lookup.mockResolvedValueOnce(IP);
    const { result } = renderHook(() => usePublicIp());

    await act(async () => {
      await result.current.check();
    });
    await waitFor(() => expect(result.current.state.status).toBe("ready"));

    const stored = Object.keys(localStorage).map((key) => localStorage.getItem(key) ?? "");
    expect(stored.some((value) => value.includes(IP))).toBe(false);
    expect(sessionStorage.length).toBe(0);
  });

  it("does not write to a component that has gone away", async () => {
    const gate = deferred<string>();
    lookup.mockReturnValue(gate.promise);
    const errors: unknown[] = [];
    const spy = vi.spyOn(console, "error").mockImplementation((...args) => errors.push(args));

    const { result, unmount } = renderHook(() => usePublicIp());
    act(() => {
      void result.current.check();
    });
    unmount();

    await act(async () => {
      gate.resolve(IP);
      await gate.promise;
    });

    expect(errors).toEqual([]);
    spy.mockRestore();
  });
});
