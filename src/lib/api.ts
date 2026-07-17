// The single API surface used by the UI. It transparently detects whether the
// app is running inside Tauri (native backend available) and, if not, falls
// back to the pure-TypeScript mock so the whole UI is developable in a browser.

import type {
  ExportFormat,
  HostResult,
  LocalNetwork,
  ScanDetail,
  ScanOptions,
  ScanProgress,
  ScanResult,
  ScanSummary,
} from "../types";
import {
  mockDelete,
  mockDetectNetworks,
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

  async scan(opts: ScanOptions, onProgress?: (p: ScanProgress) => void): Promise<ScanResult> {
    if (isTauri()) {
      let unlisten: (() => void) | undefined;
      if (onProgress) {
        const { listen } = await import("@tauri-apps/api/event");
        unlisten = await listen<ScanProgress>("scan:progress", (e) => onProgress(e.payload));
      }
      try {
        return await invoke<ScanResult>("scan_network", { opts });
      } finally {
        unlisten?.();
      }
    }
    return mockScan(opts, onProgress);
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

  async detectNetworks(): Promise<LocalNetwork[]> {
    if (isTauri()) return invoke<LocalNetwork[]>("detect_networks");
    return mockDetectNetworks();
  },

  async wakeOnLan(mac: string): Promise<void> {
    if (isTauri()) return invoke<void>("wake_on_lan", { mac });
    alert(`Wake-on-LAN magic packet to ${mac} (available in the desktop app).`);
  },

  async openSmb(ip: string): Promise<void> {
    if (isTauri()) return invoke<void>("open_smb", { ip });
    alert(`Open shared folders on ${ip} (available in the desktop app).`);
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

  // Export the given hosts in the chosen format. In Tauri this uses the native
  // save dialog and writes via the backend; in the browser it downloads a file.
  async exportHosts(
    hosts: HostResult[],
    format: ExportFormat,
    suggestedName: string,
  ): Promise<boolean> {
    const contents = buildExport(hosts, format);
    if (isTauri()) {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const path = await save({
        defaultPath: suggestedName,
        filters: [{ name: format.toUpperCase(), extensions: [format] }],
      });
      if (!path) return false;
      await invoke<void>("save_text", { path, contents });
      return true;
    }
    const mime =
      format === "json" ? "application/json" : format === "xml" ? "application/xml" : "text/csv";
    const blob = new Blob([contents], { type: `${mime};charset=utf-8` });
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

function xmlEscape(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export function buildExport(hosts: HostResult[], format: ExportFormat): string {
  if (format === "json") {
    return JSON.stringify(hosts, null, 2);
  }
  if (format === "xml") {
    const rows = hosts
      .map((h) => {
        const fields = [
          `    <ip>${xmlEscape(h.ip)}</ip>`,
          `    <hostname>${xmlEscape(h.hostname ?? "")}</hostname>`,
          `    <mac>${xmlEscape(h.mac ?? "")}</mac>`,
          `    <vendor>${xmlEscape(h.vendor ?? "")}</vendor>`,
          `    <os>${xmlEscape(h.os_guess ?? "")}</os>`,
          `    <ttl>${h.ttl ?? ""}</ttl>`,
          `    <open_ports>${xmlEscape(h.open_ports.join(" "))}</open_ports>`,
          `    <response_ms>${h.response_ms ?? ""}</response_ms>`,
          `    <last_seen>${xmlEscape(h.last_seen)}</last_seen>`,
        ].join("\n");
        return `  <host>\n${fields}\n  </host>`;
      })
      .join("\n");
    return `<?xml version="1.0" encoding="UTF-8"?>\n<hosts>\n${rows}\n</hosts>\n`;
  }
  // CSV
  const header = "IP,Hostname,MAC,Vendor,OS,TTL,Open Ports,Response (ms),Last Seen\n";
  const lines = hosts.map((h) =>
    [
      csvField(h.ip),
      csvField(h.hostname ?? ""),
      csvField(h.mac ?? ""),
      csvField(h.vendor ?? ""),
      csvField(h.os_guess ?? ""),
      csvField(h.ttl != null ? String(h.ttl) : ""),
      csvField(h.open_ports.join(" ")),
      csvField(h.response_ms != null ? String(h.response_ms) : ""),
      csvField(h.last_seen),
    ].join(","),
  );
  return header + lines.join("\n") + (lines.length ? "\n" : "");
}
