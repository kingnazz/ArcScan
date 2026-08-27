import { useCallback, useEffect, useState } from "react";
import { isTauri } from "../lib/api";

export type UpdateStatus =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "installing"
  | "uptodate"
  | "error";

// Minimal shape of the object returned by @tauri-apps/plugin-updater `check()`.
interface UpdateHandle {
  version: string;
  body?: string;
  downloadAndInstall: (
    onEvent?: (e: { event: string; data?: { contentLength?: number; chunkLength?: number } }) => void,
  ) => Promise<void>;
}

/**
 * In-app auto-updater.
 *
 * Installed builds check the configured update feed and can download, install
 * and relaunch. Manual (Portable) mode never imports or calls either native
 * updater plugin; its fixed downloads-page action lives outside this hook.
 *
 * The check contacts GitHub, so it is listed in the privacy documentation
 * alongside the public-IP lookup and can be switched off in Settings. Unlike the
 * public-IP lookup it stays on by default: an out-of-date network tool is a
 * problem in itself, and the request carries only the version being checked.
 */
export function useUpdater(autoCheck = true, mode: "installer" | "manual" = "installer") {
  const [status, setStatus] = useState<UpdateStatus>("idle");
  const [version, setVersion] = useState<string | null>(null);
  const [notes, setNotes] = useState<string | null>(null);
  const [progress, setProgress] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [handle, setHandle] = useState<UpdateHandle | null>(null);

  const check = useCallback(async (manual = false) => {
    if (mode === "manual") return;
    if (!isTauri()) return;
    setError(null);
    setStatus("checking");
    try {
      const { check } = await import("@tauri-apps/plugin-updater");
      const update = (await check()) as unknown as UpdateHandle | null;
      if (update) {
        setHandle(update);
        setVersion(update.version);
        setNotes(update.body ?? null);
        setStatus("available");
      } else {
        setStatus(manual ? "uptodate" : "idle");
      }
    } catch (e) {
      // Offline / no feed yet / not configured — stay quiet unless the user
      // explicitly asked.
      setError(e instanceof Error ? e.message : String(e));
      setStatus(manual ? "error" : "idle");
    }
  }, [mode]);

  useEffect(() => {
    if (autoCheck && mode === "installer") check(false);
  }, [check, autoCheck, mode]);

  const install = useCallback(async () => {
    // The frontend half of the Portable gate. The binary also omits both
    // updater plugins, so neither layer has an install-and-relaunch path.
    if (mode === "manual") return;
    if (!handle) return;
    try {
      setStatus("downloading");
      let downloaded = 0;
      let total = 0;
      await handle.downloadAndInstall((e) => {
        if (e.event === "Started") {
          total = e.data?.contentLength ?? 0;
        } else if (e.event === "Progress") {
          downloaded += e.data?.chunkLength ?? 0;
          if (total > 0) setProgress(Math.min(100, Math.round((downloaded / total) * 100)));
        } else if (e.event === "Finished") {
          setStatus("installing");
        }
      });
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setStatus("error");
    }
  }, [handle, mode]);

  const dismiss = useCallback(() => setStatus("idle"), []);

  return { status, version, notes, progress, error, check, install, dismiss, mode };
}
