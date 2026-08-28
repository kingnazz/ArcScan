import { useEffect, useState } from "react";
import { api } from "../lib/api";
import type { RuntimeInfo } from "../lib/runtime";

/**
 * Which edition this is, asked once at startup.
 *
 * Null until the backend answers. Edition-specific controls stay hidden and the
 * updater hook defaults to manual/no-op during that interval, so a failed read
 * cannot make a Portable process call the Installer updater.
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
