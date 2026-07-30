import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../lib/api";

export type PublicIpState =
  | { status: "idle" }
  | { status: "checking" }
  | { status: "ready"; ip: string }
  | { status: "error"; message: string };

/**
 * The machine's public address, looked up only when asked.
 *
 * v1.6 fetched this from a third-party service on every launch, which meant
 * ArcScan made an outbound request before the operator had done anything. Now
 * nothing happens until [`check`] is called from the explicit action, the result
 * is held in memory for the session only, and closing the app forgets it.
 */
export function usePublicIp() {
  const [state, setState] = useState<PublicIpState>({ status: "idle" });
  const controller = useRef<AbortController | null>(null);

  const check = useCallback(async () => {
    controller.current?.abort();
    const next = new AbortController();
    controller.current = next;
    setState({ status: "checking" });

    // A hung request must not leave the button spinning forever.
    const timer = setTimeout(() => next.abort(), 8_000);
    try {
      const ip = await api.publicIp(next.signal);
      setState({ status: "ready", ip });
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") {
        setState({ status: "idle" });
        return;
      }
      setState({
        status: "error",
        message: error instanceof Error ? error.message : "The lookup failed.",
      });
    } finally {
      clearTimeout(timer);
    }
  }, []);

  const clear = useCallback(() => {
    controller.current?.abort();
    setState({ status: "idle" });
  }, []);

  useEffect(() => () => controller.current?.abort(), []);

  return { state, check, clear };
}
