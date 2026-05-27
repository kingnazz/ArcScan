// Thin abstraction over the Tauri backend. When ArcScan runs inside the Tauri
// runtime it dispatches real `invoke` calls and subscribes to scan events; when
// it runs in a plain browser (vite dev) it transparently uses the mock scanner
// so the UI is fully functional during frontend development.

import type {
  Host,
  ScanOptions,
  ScanProgress,
  ScanResult,
  ScanSummary,
} from "../types";
import { mockScan } from "./mock";

export const isTauri = (): boolean =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export type LaunchKind = "web" | "rdp" | "ssh";

interface ScanCallbacks {
  onProgress?: (p: ScanProgress) => void;
  onHost?: (h: Host) => void;
}

// ---- Tauri-only dynamic imports -------------------------------------------
// Imported lazily so the mock path never pulls the Tauri API in a browser.

async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

async function tauriListen<T>(event: string, handler: (payload: T) => void) {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<T>(event, (e) => handler(e.payload));
}

// ---- Public API ------------------------------------------------------------

export async function runScan(
  options: ScanOptions,
  callbacks: ScanCallbacks = {}
): Promise<ScanResult> {
  if (!isTauri()) {
    return mockScan(
      options,
      callbacks.onProgress ?? (() => {}),
      callbacks.onHost ?? (() => {})
    );
  }

  const unlisteners: Array<() => void> = [];
  if (callbacks.onProgress) {
    unlisteners.push(await tauriListen<ScanProgress>("scan://progress", callbacks.onProgress));
  }
  if (callbacks.onHost) {
    unlisteners.push(await tauriListen<Host>("scan://host", callbacks.onHost));
  }

  try {
    return await tauriInvoke<ScanResult>("scan_network", { options });
  } finally {
    unlisteners.forEach((u) => u());
  }
}

export async function cancelScan(): Promise<void> {
  if (!isTauri()) return;
  await tauriInvoke<void>("cancel_scan");
}

export async function listScans(): Promise<ScanSummary[]> {
  if (!isTauri()) return [];
  return tauriInvoke<ScanSummary[]>("list_scans");
}

export async function getScanHosts(scanId: number): Promise<Host[]> {
  if (!isTauri()) return [];
  return tauriInvoke<Host[]>("get_scan_hosts", { scanId });
}

export async function deleteScan(scanId: number): Promise<void> {
  if (!isTauri()) return;
  await tauriInvoke<void>("delete_scan", { scanId });
}

export async function launchAction(
  kind: LaunchKind,
  ip: string,
  port?: number
): Promise<void> {
  if (!isTauri()) {
    if (kind === "web") {
      const scheme = port === 443 || port === 8443 ? "https" : "http";
      const portPart = port && port !== 80 && port !== 443 ? `:${port}` : "";
      window.open(`${scheme}://${ip}${portPart}`, "_blank");
    } else {
      alert(`${kind.toUpperCase()} launch is only available in the desktop app.`);
    }
    return;
  }
  await tauriInvoke<void>("launch_action", { kind, ip, port: port ?? null });
}

export async function copyToClipboard(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // Fallback for environments without the async clipboard API.
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    document.execCommand("copy");
    document.body.removeChild(ta);
  }
}

/** Save CSV text to disk via a native dialog. Returns the saved path or null. */
export async function exportCsv(csv: string, suggestedName: string): Promise<string | null> {
  if (!isTauri()) {
    const { downloadCsv } = await import("./csv");
    downloadCsv(suggestedName, csv);
    return suggestedName;
  }
  const { save } = await import("@tauri-apps/plugin-dialog");
  const path = await save({
    defaultPath: suggestedName,
    filters: [{ name: "CSV", extensions: ["csv"] }],
  });
  if (!path) return null;
  await tauriInvoke<void>("write_text_file", { path, contents: csv });
  return path;
}
