import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import { PUBLIC_IP_TIMEOUT_MS, PublicIpError, isAbortError } from "../lib/publicIp";

export type PublicIpState =
  | { status: "idle" }
  | { status: "checking" }
  | { status: "ready"; ip: string; checkedAt: number }
  | { status: "error"; message: string; technical?: string };

/**
 * The machine's public address, looked up only when asked.
 *
 * v1.6 fetched this from a third-party service on every launch, which meant
 * ArcScan made an outbound request before the operator had done anything.
 * Nothing happens here until [`check`] is called from an explicit Check,
 * Refresh or Retry: this hook installs no effect that starts a lookup, so
 * mounting it, switching views and running a scan all cost nothing.
 *
 * The result is held in memory for the session only. It is never written to the
 * database, never written to localStorage, and never included in an export, so
 * closing the app forgets it.
 */
export function usePublicIp() {
  const [state, setState] = useState<PublicIpState>({ status: "idle" });
  const controller = useRef<AbortController | null>(null);
  /**
   * Increments on every lookup and on every clear. A response whose id is no
   * longer the current one is stale — the operator has since forgotten the
   * value or started a newer lookup — and must not be written to the screen.
   */
  const requestId = useRef(0);
  const inFlight = useRef(false);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      controller.current?.abort();
    };
  }, []);

  const check = useCallback(async () => {
    // A second press while a lookup is running is a duplicate, not a new
    // request. Honouring it would mean two outbound requests for one answer.
    if (inFlight.current) return;
    inFlight.current = true;

    const id = ++requestId.current;
    const next = new AbortController();
    controller.current = next;
    setState({ status: "checking" });

    // A hung provider must not leave the control spinning forever. Tracked
    // separately from the abort itself, because a timeout is a failure worth
    // reporting while a cancellation is not.
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      next.abort();
    }, PUBLIC_IP_TIMEOUT_MS);

    try {
      const ip = await api.publicIp(next.signal);
      if (id !== requestId.current || !mounted.current) return;
      setState({ status: "ready", ip, checkedAt: Date.now() });
    } catch (error) {
      if (id !== requestId.current || !mounted.current) return;
      if (isAbortError(error)) {
        if (timedOut) {
          setState({
            status: "error",
            message: "The lookup timed out.",
            technical: `No provider answered within ${Math.round(PUBLIC_IP_TIMEOUT_MS / 1000)} seconds.`,
          });
          return;
        }
        // Cancelled some other way, and this is still the current request:
        // nothing was learned, so go back to Not checked rather than leaving
        // the control spinning on a lookup that will never answer. A cancel
        // through `clear` never reaches here, because clearing invalidates the
        // request id above and sets its own state.
        setState({ status: "idle" });
        return;
      }
      setState({
        status: "error",
        message: error instanceof Error ? error.message : "The lookup failed.",
        technical: error instanceof PublicIpError ? error.technical : undefined,
      });
    } finally {
      clearTimeout(timer);
      if (id === requestId.current) inFlight.current = false;
    }
  }, []);

  /** Forget the address and cancel anything in flight. */
  const clear = useCallback(() => {
    requestId.current += 1;
    inFlight.current = false;
    controller.current?.abort();
    controller.current = null;
    setState({ status: "idle" });
  }, []);

  return { state, check, clear };
}
