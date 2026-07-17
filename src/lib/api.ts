// The single API surface used by the UI. It transparently detects whether the
// app is running inside Tauri (native backend available) and, if not, falls
// back to the pure-TypeScript mock so the whole UI is developable in a browser.

import type {
  HostResult,
  ScanDetail,
  ScanOptions,
  ScanResult,
  ScanSummary,
} from "../types";
import {
  mockDelete,
  mockGet,
  mockLastIps,
  mockList,
  mockSave,
  mockScan,
} from "./mock";

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

export const api = {
  native: isTauri(),

  async scan(opts: ScanOptions): Promise<ScanResult> {
    if (isTauri()) return invoke<ScanResult>("scan_network", { opts });
    return mockScan(opts);
  },

  async save(result: ScanResult): Promise<number> {
    if (isTauri()) return invoke<number>("save_scan", { result });
    return mockSave(result);
  },

  async listScans(): Promise<ScanSummary[]> {
    if (isTauri()) return invoke<ScanSummary[]>("list_scans");
    return mockList();
  },

  async getScan(id: number): Promise<ScanDetail> {
    if (isTauri()) return invoke<ScanDetail>("get_scan", { id });
    return mockGet(id);
  },

  async deleteScan(id: number): Promise<void> {
    if (isTauri()) return invoke<void>("delete_scan", { id });
    mockDelete(id);
  },

  async lastScanIps(): Promise<string[]> {
    if (isTauri()) return invoke<string[]>("last_scan_ips");
    return mockLastIps();
  },

  async openWeb(ip: string, port?: number): Promise<void> {
    if (isTauri()) return invoke<void>("open_web", { ip, port: port ?? null });
    window.open(port && port !== 80 && port !== 443 ? `http://${ip}:${port}` : `http://${ip}`, "_blank");
  },

  async openRdp(ip: string): Promise<void> {
    if (isTauri()) return invoke<void>("open_rdp", { ip });
    alert(`RDP to ${ip} (available in the desktop app).`);
  },

  async openSsh(ip: string): Promise<void> {
    if (isTauri()) return invoke<void>("open_ssh", { ip });
    alert(`SSH to ${ip} (available in the desktop app).`);
  },

  async copyIp(ip: string): Promise<void> {
    await navigator.clipboard.writeText(ip);
  },

  // Export the given hosts to CSV. In Tauri this uses the native save dialog
  // and writes via the backend; in the browser it triggers a file download.
  async exportCsv(hosts: HostResult[], suggestedName: string): Promise<boolean> {
    if (isTauri()) {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const path = await save({
        defaultPath: suggestedName,
        filters: [{ name: "CSV", extensions: ["csv"] }],
      });
      if (!path) return false;
      await invoke<void>("export_csv", { path, hosts });
      return true;
    }
    const csv = buildCsv(hosts);
    const blob = new Blob([csv], { type: "text/csv;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = suggestedName;
    a.click();
    URL.revokeObjectURL(url);
    return true;
  },
};

function csvField(s: string): string {
  if (s.includes(",") || s.includes('"') || s.includes("\n")) {
    return `"${s.replace(/"/g, '""')}"`;
  }
  return s;
}

export function buildCsv(hosts: HostResult[]): string {
  const header = "IP,Hostname,MAC,Vendor,Open Ports,Response (ms),Last Seen\n";
  const rows = hosts.map((h) =>
    [
      csvField(h.ip),
      csvField(h.hostname ?? ""),
      csvField(h.mac ?? ""),
      csvField(h.vendor ?? ""),
      csvField(h.open_ports.join(" ")),
      csvField(h.response_ms != null ? String(h.response_ms) : ""),
      csvField(h.last_seen),
    ].join(","),
  );
  return header + rows.join("\n") + (rows.length ? "\n" : "");
}
