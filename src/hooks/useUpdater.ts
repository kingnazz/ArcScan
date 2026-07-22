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
 * In-app auto-updater. On launch it silently checks the configured update feed;
 * if a newer signed build is available it exposes it so the UI can offer a
 * one-click "Update now" (download → install → relaunch). No-ops outside Tauri
 * or when no update feed is reachable.
 */
export function useUpdater() {
  const [status, setStatus] = useState<UpdateStatus>("idle");
  const [version, setVersion] = useState<string | null>(null);
  const [notes, setNotes] = useState<string | null>(null);
  const [progress, setProgress] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [handle, setHandle] = useState<UpdateHandle | null>(null);

  const check = useCallback(async (manual = false) => {
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
  }, []);

  useEffect(() => {
    check(false);
  }, [check]);

  const install = useCallback(async () => {
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
  }, [handle]);

  const dismiss = useCallback(() => setStatus("idle"), []);

  return { status, version, notes, progress, error, check, install, dismiss };
}
