// The single API surface used by the UI.
//
// It detects whether the app is running inside Tauri and, if not, falls back to
// the pure-TypeScript mock, so the whole interface is developable, testable and
// screenshotable in a browser without a Rust build.

import type {
  BulkOutcome,
  ChangeFeed,
  ChangeState,
  Device,
  DeviceDetail,
  DeviceStatus,
  ExportFormat,
  HostEvent,
  HostRemovedEvent,
  InventorySummary,
  LocalNetwork,
  NetworkScope,
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
import type { ChangeEvent, InventoryRow } from "../types";
import type { DeviceRow } from "./live";
import {
  buildChangesExport,
  buildExport,
  buildInventoryExport,
  datedFilename,
  exportFilename,
} from "./export";
import { mock } from "./mock";
import { lookupPublicIp } from "./publicIp";
import type { RuntimeInfo } from "./runtime";
import type { ArcAtlasConnection, ArcAtlasHandoffEnvelope, ArcAtlasSendResult } from "./arcatlas";

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

  /** The persistent Inventory: one row per device, across every scan. */
  async inventory(): Promise<InventorySummary> {
    if (isTauri()) return invoke<InventorySummary>("inventory_summary");
    return mock.inventory();
  },

  /** The Changes inbox, newest first. */
  async changeEvents(): Promise<ChangeFeed> {
    if (isTauri()) return invoke<ChangeFeed>("list_change_events");
    return mock.changeEvents();
  },

  /** Acknowledge, ignore or reopen change events. */
  async setChangeState(ids: number[], state: ChangeState): Promise<BulkOutcome> {
    if (isTauri()) return invoke<BulkOutcome>("set_change_state", { ids, state });
    return mock.setChangeState(ids, state);
  },

  /** Classify several devices at once, for the Inventory's bulk actions. */
  async setDeviceStatuses(ids: number[], status: DeviceStatus): Promise<BulkOutcome> {
    if (isTauri()) return invoke<BulkOutcome>("set_device_statuses", { ids, status });
    return mock.setDeviceStatuses(ids, status);
  },

  async listNetworkScopes(): Promise<NetworkScope[]> {
    if (isTauri()) return invoke<NetworkScope[]>("list_network_scopes");
    return mock.listNetworkScopes();
  },

  async renameNetworkScope(id: number, name: string): Promise<void> {
    if (isTauri()) return invoke<void>("rename_network_scope", { id, name });
    mock.renameNetworkScope(id, name);
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

  /**
   * Correct, change or clear ArcScan's detected device type for one device.
   *
   * `null` restores Auto. An explicit `"unknown"` is a real answer and is
   * stored as one. The backend validates the value and refuses anything that is
   * not a shipped type, so a save failure here is a message the caller shows
   * rather than a value quietly recorded.
   */
  async setDeviceTypeOverride(id: number, deviceType: string | null): Promise<void> {
    if (isTauri()) return invoke<void>("set_device_type_override", { id, deviceType });
    mock.setDeviceTypeOverride(id, deviceType);
  },

  /**
   * The redacted discovery report for one device, for the clipboard.
   *
   * Contacts nothing and writes nothing. What it omits is enforced where it is
   * built: in Rust for the packaged app, and in `lib/diagnostics` for the
   * browser demo.
   */
  async deviceDiscoveryReport(id: number): Promise<string> {
    if (isTauri()) return invoke<string>("device_discovery_report", { id });
    return mock.deviceDiscoveryReport(id);
  },

  /** Note bodies for the devices an export covers. */
  async deviceNotes(ids: number[]): Promise<Map<number, string>> {
    const pairs = isTauri()
      ? await invoke<Array<[number, string]>>("device_notes", { ids })
      : mock.deviceNotes(ids);
    return new Map(pairs);
  },

  async importDeviceLabels(labels: Record<string, string>): Promise<number> {
    if (isTauri()) return invoke<number>("import_device_labels", { labels });
    return mock.importDeviceLabels(labels);
  },

  async detectNetworks(): Promise<LocalNetwork[]> {
    if (isTauri()) return invoke<LocalNetwork[]>("detect_networks");
    return mock.detectNetworks();
  },

  /**
   * Which edition this is and whether its state is persistent or temporary.
   *
   * Answered by Rust from the paths it resolved at startup. The browser demo
   * answers from the mock, which reports an installed edition unless a test
   * asks otherwise -- see lib/mock.ts. Nothing here can change where the
   * native app actually stores anything.
   */
  async runtimeInfo(): Promise<RuntimeInfo> {
    if (isTauri()) return invoke<RuntimeInfo>("runtime_info");
    return mock.runtimeInfo();
  },

  /**
   * Reveal Installed ArcScan's data folder. Portable rejects this command and
   * never exposes its disposable internal path.
   */
  async openDataFolder(): Promise<void> {
    if (isTauri()) return invoke<void>("open_data_folder");
    throw new Error("Opening the data folder is only available in the ArcScan desktop app.");
  },

  /** Open the download page, for a portable copy that needs a newer ZIP. */
  async openPortableDownloads(): Promise<void> {
    if (isTauri()) return invoke<void>("open_portable_downloads");
    window.open("https://kingnazz.github.io/ArcScan/#download", "_blank", "noopener");
  },

  async openReleases(): Promise<void> {
    if (isTauri()) return invoke<void>("open_releases");
    window.open("https://github.com/kingnazz/ArcScan/releases", "_blank", "noopener");
  },

  /**
   * Open the privacy notes in the system browser. Routed through a Rust
   * command with a fixed URL: the strict CSP means the webview itself cannot
   * navigate to external origins.
   */
  async openPrivacy(): Promise<void> {
    if (isTauri()) return invoke<void>("open_privacy");
    window.open("https://kingnazz.github.io/ArcScan/privacy.html", "_blank", "noopener");
  },

  /**
   * Look up this machine's public address.
   *
   * Only ever called from an explicit Check, Refresh or Retry. It contacts a
   * third-party provider and sends nothing but the request itself: no scan
   * target, no result, no device data.
   *
   * The browser build answers from scripted providers instead of the real ones,
   * so the demo, the screenshots and the browser suite never make an outbound
   * request and never display a real person's address. The fallback logic
   * itself is the same code in both builds.
   */
  async publicIp(signal?: AbortSignal): Promise<string> {
    if (!isTauri()) return mock.publicIp(signal);
    return lookupPublicIp(fetch, signal);
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
    return writeExport(buildExport(rows, format), exportFilename(target, format), format);
  },

  /**
   * Export Inventory rows. The caller decides the scope — everything, the
   * current filter, the selection or one network — and passes the label that
   * names it, which becomes part of the filename.
   */
  async exportInventory(
    rows: InventoryRow[],
    format: ExportFormat,
    scopeLabel: string | null,
  ): Promise<boolean> {
    const notes = await api.deviceNotes(rows.filter((r) => r.notes_present).map((r) => r.device_id));
    return writeExport(
      buildInventoryExport(rows, format, notes),
      datedFilename("inventory", scopeLabel, format),
      format,
    );
  },

  /** Export change events, exactly the ones the caller passes in. */
  async exportChanges(
    events: ChangeEvent[],
    format: ExportFormat,
    scopeLabel: string | null,
  ): Promise<boolean> {
    return writeExport(
      buildChangesExport(events, format),
      datedFilename("changes", scopeLabel, format),
      format,
    );
  },

  async getArcAtlasConnection(): Promise<ArcAtlasConnection> {
    if (isTauri()) return invoke<ArcAtlasConnection>("get_arcatlas_connection");
    return mock.getArcAtlasConnection();
  },

  async configureArcAtlasConnection(serverUrl: string, token: string): Promise<ArcAtlasConnection> {
    if (isTauri()) {
      return invoke<ArcAtlasConnection>("configure_arcatlas_connection", { serverUrl, token });
    }
    return mock.configureArcAtlasConnection(serverUrl, token);
  },

  async disconnectArcAtlasConnection(): Promise<ArcAtlasConnection> {
    if (isTauri()) return invoke<ArcAtlasConnection>("disconnect_arcatlas_connection");
    return mock.disconnectArcAtlasConnection();
  },

  async sendInventoryToArcAtlas(envelope: ArcAtlasHandoffEnvelope): Promise<ArcAtlasSendResult> {
    if (isTauri()) {
      return invoke<ArcAtlasSendResult>("send_inventory_to_arcatlas", { envelope });
    }
    return mock.sendInventoryToArcAtlas(envelope);
  },

  async openArcAtlas(url: string): Promise<void> {
    if (isTauri()) return invoke<void>("open_arcatlas_url", { url });
    window.open(url, "_blank", "noopener");
  },
};

/**
 * Write already-formatted export text, through the native save dialog when
 * ArcScan is running as a desktop app and through a download otherwise.
 *
 * Returns false when the operator dismissed the dialog, which is a cancellation
 * rather than a failure.
 */
async function writeExport(
  contents: string,
  suggestedName: string,
  format: ExportFormat,
): Promise<boolean> {
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
}
