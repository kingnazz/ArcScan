// The single API surface used by the UI.
//
// It detects whether the app is running inside Tauri and, if not, falls back to
// the pure-TypeScript mock, so the whole interface is developable, testable and
// screenshotable in a browser without a Rust build.

import type {
  Device,
  DeviceDetail,
  DeviceStatus,
  ExportFormat,
  HostEvent,
  HostRemovedEvent,
  LocalNetwork,
  SavedScan,
  ScanComparison,
  ScanDetail,
  ScanOptions,
  ScanPreview,
  ScanProgress,
  ScanResult,
  ScanStarted,
  ScanSummary,
  ServiceInfo,
} from "../types";
import type { DeviceRow } from "./live";
import { buildExport, exportFilename } from "./export";
import { mock } from "./mock";

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

/** The callbacks a caller supplies to receive a scan's streamed events. */
export interface ScanListeners {
  onStarted?: (started: ScanStarted) => void;
  onProgress?: (progress: ScanProgress) => void;
  onHostDiscovered?: (event: HostEvent) => void;
  onHostUpdated?: (event: HostEvent) => void;
  onHostRemoved?: (event: HostRemovedEvent) => void;
}

/** Public-IP lookup providers, in the order they are tried. */
export const PUBLIC_IP_PROVIDERS = [
  { name: "ipify", url: "https://api64.ipify.org?format=json", json: true },
  { name: "icanhazip", url: "https://icanhazip.com", json: false },
] as const;

function looksLikeIp(value: string): boolean {
  return /^(\d{1,3}\.){3}\d{1,3}$/.test(value) || (/:/.test(value) && /^[0-9a-fA-F:.]+$/.test(value));
}

export const api = {
  native: isTauri(),

  /**
   * Run a scan. Events are subscribed to before the command is invoked, so no
   * host discovered in the first milliseconds can be missed.
   */
  async scan(opts: ScanOptions, listeners: ScanListeners = {}): Promise<ScanResult> {
    if (!isTauri()) return mock.scan(opts, listeners);

    const { listen } = await import("@tauri-apps/api/event");
    const unlisten: Array<() => void> = [];
    const subscribe = async <T,>(name: string, handler: ((payload: T) => void) | undefined) => {
      if (!handler) return;
      unlisten.push(await listen<T>(name, (e) => handler(e.payload)));
    };

    await Promise.all([
      subscribe<ScanStarted>("scan:started", listeners.onStarted),
      subscribe<ScanProgress>("scan:progress", listeners.onProgress),
      subscribe<HostEvent>("scan:host-discovered", listeners.onHostDiscovered),
      subscribe<HostEvent>("scan:host-updated", listeners.onHostUpdated),
      subscribe<HostRemovedEvent>("scan:host-removed", listeners.onHostRemoved),
    ]);

    try {
      return await invoke<ScanResult>("scan_network", { opts });
    } finally {
      for (const off of unlisten) off();
    }
  },

  /** Ask the running scan to stop. It resolves with the hosts found so far. */
  async cancelScan(): Promise<void> {
    if (isTauri()) return invoke<void>("cancel_scan");
    mock.cancelScan();
  },

  /** What a scan would do, so the workload can be shown before it starts. */
  async previewScan(opts: ScanOptions): Promise<ScanPreview> {
    if (isTauri()) return invoke<ScanPreview>("preview_scan", { opts });
    return mock.previewScan(opts);
  },

  /** Parse a port specification using the backend's rules. */
  async parsePortSpec(spec: string): Promise<number[]> {
    if (isTauri()) return invoke<number[]>("parse_port_spec", { spec });
    return mock.parsePortSpec(spec);
  },

  async serviceCatalog(): Promise<ServiceInfo[]> {
    if (isTauri()) return invoke<ServiceInfo[]>("service_catalog");
    return mock.serviceCatalog();
  },

  async save(result: ScanResult): Promise<SavedScan> {
    if (isTauri()) return invoke<SavedScan>("save_scan", { result });
    return mock.save(result);
  },

  async listScans(): Promise<ScanSummary[]> {
    if (isTauri()) return invoke<ScanSummary[]>("list_scans");
    return mock.listScans();
  },

  async getScan(id: number): Promise<ScanDetail> {
    if (isTauri()) return invoke<ScanDetail>("get_scan", { id });
    return mock.getScan(id);
  },

  async compareScan(id: number): Promise<ScanComparison> {
    if (isTauri()) return invoke<ScanComparison>("compare_scan", { id });
    return mock.compareScan(id);
  },

  async deleteScan(id: number): Promise<void> {
    if (isTauri()) return invoke<void>("delete_scan", { id });
    mock.deleteScan(id);
  },

  async pruneHistory(keep: number): Promise<number> {
    if (isTauri()) return invoke<number>("prune_history", { keep });
    return mock.pruneHistory(keep);
  },

  async listDevices(): Promise<Device[]> {
    if (isTauri()) return invoke<Device[]>("list_devices");
    return mock.listDevices();
  },

  async deviceDetail(id: number): Promise<DeviceDetail> {
    if (isTauri()) return invoke<DeviceDetail>("device_detail", { id });
    return mock.deviceDetail(id);
  },

  async setDeviceName(id: number, name: string | null): Promise<void> {
    if (isTauri()) return invoke<void>("set_device_name", { id, name });
    mock.setDeviceName(id, name);
  },

  async setDeviceStatus(id: number, status: DeviceStatus): Promise<void> {
    if (isTauri()) return invoke<void>("set_device_status", { id, status });
    mock.setDeviceStatus(id, status);
  },

  async setDeviceNotes(id: number, notes: string | null): Promise<void> {
    if (isTauri()) return invoke<void>("set_device_notes", { id, notes });
    mock.setDeviceNotes(id, notes);
  },

  async importDeviceLabels(labels: Record<string, string>): Promise<number> {
    if (isTauri()) return invoke<number>("import_device_labels", { labels });
    return mock.importDeviceLabels(labels);
  },

  async detectNetworks(): Promise<LocalNetwork[]> {
    if (isTauri()) return invoke<LocalNetwork[]>("detect_networks");
    return mock.detectNetworks();
  },

  async openReleases(): Promise<void> {
    if (isTauri()) return invoke<void>("open_releases");
    window.open("https://github.com/kingnazz/ArcScan/releases", "_blank", "noopener");
  },

  /**
   * Look up this machine's public address.
   *
   * Only ever called from the explicit "Check public IP" action. It contacts a
   * third-party service and sends nothing but the request itself: no scan
   * target, no result, no device data.
   */
  async publicIp(signal?: AbortSignal): Promise<string> {
    for (const provider of PUBLIC_IP_PROVIDERS) {
      try {
        const response = await fetch(provider.url, { signal });
        if (!response.ok) continue;
        const value = provider.json
          ? String(((await response.json()) as { ip?: unknown }).ip ?? "").trim()
          : (await response.text()).trim();
        if (looksLikeIp(value)) return value;
      } catch (error) {
        // An aborted request is the operator navigating away, not a failure.
        if (error instanceof DOMException && error.name === "AbortError") throw error;
      }
    }
    throw new Error(
      "Could not reach a public-IP service. Check your internet connection and try again.",
    );
  },

  async wakeOnLan(mac: string): Promise<void> {
    if (isTauri()) return invoke<void>("wake_on_lan", { mac });
    throw new Error("Wake-on-LAN is only available in the ArcScan desktop app.");
  },

  async openSmb(ip: string): Promise<void> {
    if (isTauri()) return invoke<void>("open_smb", { ip });
    throw new Error("Opening shared folders is only available in the ArcScan desktop app.");
  },

  async openWeb(ip: string, port?: number): Promise<void> {
    if (isTauri()) return invoke<void>("open_web", { ip, port: port ?? null });
    const scheme = port === 443 || port === 8443 ? "https" : "http";
    const url =
      port && port !== 80 && port !== 443 ? `${scheme}://${ip}:${port}` : `${scheme}://${ip}`;
    window.open(url, "_blank", "noopener");
  },

  async openRdp(ip: string): Promise<void> {
    if (isTauri()) return invoke<void>("open_rdp", { ip });
    throw new Error("Remote Desktop is only available in the ArcScan desktop app.");
  },

  async openSsh(ip: string): Promise<void> {
    if (isTauri()) return invoke<void>("open_ssh", { ip });
    throw new Error("SSH is only available in the ArcScan desktop app.");
  },

  async copyText(text: string): Promise<void> {
    await navigator.clipboard.writeText(text);
  },

  /**
   * Export rows in the chosen format. Returns false when the operator dismissed
   * the save dialog, which is a cancellation rather than a failure.
   */
  async exportRows(rows: DeviceRow[], format: ExportFormat, target: string): Promise<boolean> {
    const contents = buildExport(rows, format);
    const suggestedName = exportFilename(target, format);

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
    const link = document.createElement("a");
    link.href = url;
    link.download = suggestedName;
    link.click();
    URL.revokeObjectURL(url);
    return true;
  },
};
