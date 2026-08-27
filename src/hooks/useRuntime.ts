import { useEffect, useState } from "react";
import { api } from "../lib/api";
import type { RuntimeInfo } from "../lib/runtime";

/**
 * Which edition this is, asked once at startup.
 *
 * Null until the backend answers. Every caller treats null as "not portable",
 * which is the safe reading in both directions: an installed build is what
 * every release before 1.8.4 was, so a moment of null shows the interface those
 * releases showed, and a portable build cannot have its updater re-enabled by a
 * failed read because the portable binary does not contain one.
 */
export function useRuntime(): RuntimeInfo | null {
  const [info, setInfo] = useState<RuntimeInfo | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .runtimeInfo()
      .then((next) => {
        if (!cancelled) setInfo(next);
      })
      .catch(() => {
        // An edition ArcScan cannot name is not worth a visible error: the
        // interface simply does not show the edition line.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return info;
}
