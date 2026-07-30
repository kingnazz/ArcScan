// Pure-TypeScript mock backend.
//
// Runs entirely in the browser with no Rust build, so the whole interface is
// developable, reviewable and screenshotable without installing a toolchain. The
// API layer falls back to it automatically when the app is not inside Tauri.
//
// It mirrors the real backend's behaviour rather than returning placeholders: it
// streams host events with the same shapes and ordering, resolves device identity
// by MAC, and computes the same comparison. Every value is deliberately fictional
// so a screenshot can never expose a real network. The native app never uses it.

import type {
  ChangeKind,
  Device,
  DeviceDetail,
  DeviceDiff,
  DeviceObservation,
  DeviceStatus,
  FieldChange,
  HostResult,
  LocalNetwork,
  SavedScan,
  ScanComparison,
  ScanDetail,
  ScanOptions,
  ScanPreview,
  ScanResult,
  ScanSummary,
  ServiceInfo,
} from "../types";
import type { ScanListeners } from "./api";
import { DEFAULT_PORTS } from "./profiles";
import { parsePorts, serviceWithPort } from "./format";

const DEMO_CIDR = "192.168.10.0/24";

/** One device on the fictional demo network. */
interface DemoDevice {
  ip: string;
  mac: string;
  vendor: string;
  hostname: string | null;
  ports: number[];
  ttl: number;
  icmp: number;
  name?: string;
  status?: DeviceStatus;
  notes?: string;
}

/**
 * A small-office network: a gateway, a couple of servers, workstations, a
 * printer, access points, cameras and some IoT. Chosen so the table exercises
 * every state the UI has to handle, including hosts with no name at all.
 */
const DEMO_NETWORK: DemoDevice[] = [
  {
    ip: "192.168.10.1",
    mac: "F4:92:BF:1A:0C:31",
    vendor: "Ubiquiti Inc.",
    hostname: "gateway.office.lan",
    ports: [53, 80, 443],
    ttl: 64,
    icmp: 0.84,
    name: "Office Gateway",
    status: "trusted",
    notes: "Primary router. Firmware reviewed quarterly.",
  },
  {
    ip: "192.168.10.4",
    mac: "00:11:32:5D:A2:77",
    vendor: "Synology Incorporated",
    hostname: "nas-backup",
    ports: [22, 443, 445, 5000],
    ttl: 64,
    icmp: 1.2,
    name: "Backup NAS",
    status: "trusted",
    notes: "Nightly backup target for the file server.",
  },
  {
    ip: "192.168.10.8",
    mac: "00:15:5D:3C:11:9E",
    vendor: "Microsoft Corporation",
    hostname: "srv-files01",
    ports: [135, 139, 445, 3389],
    ttl: 128,
    icmp: 1.05,
    name: "File Server",
    status: "known",
  },
  {
    ip: "192.168.10.12",
    mac: "3C:22:FB:88:41:D0",
    vendor: "Apple, Inc.",
    hostname: "reception-imac",
    ports: [22, 548, 5900],
    ttl: 64,
    icmp: 2.4,
    status: "known",
  },
  {
    ip: "192.168.10.19",
    mac: "18:66:DA:70:2B:14",
    vendor: "Dell Inc.",
    hostname: "ws-accounting",
    ports: [135, 139, 445],
    ttl: 128,
    icmp: 3.1,
  },
  {
    ip: "192.168.10.23",
    mac: "94:C6:91:0E:5A:88",
    vendor: "Intel Corporate",
    hostname: null,
    ports: [],
    ttl: 128,
    icmp: 6.7,
  },
  {
    ip: "192.168.10.31",
    mac: "B8:27:EB:44:19:F2",
    vendor: "Raspberry Pi Foundation",
    hostname: "monitoring-pi",
    ports: [22, 80, 3000],
    ttl: 64,
    icmp: 1.6,
    name: "Monitoring Pi",
    status: "watched",
    notes: "Runs the uptime dashboard. Watch for new listening ports.",
  },
  {
    ip: "192.168.10.44",
    mac: "F4:92:BF:2C:71:A9",
    vendor: "Ubiquiti Inc.",
    hostname: "ap-warehouse",
    ports: [22, 443],
    ttl: 64,
    icmp: 4.2,
    name: "Warehouse Access Point",
    status: "trusted",
  },
  {
    ip: "192.168.10.57",
    mac: "3C:D9:2B:6F:08:AA",
    vendor: "Hewlett Packard",
    hostname: "front-office-printer",
    ports: [80, 443, 515, 631, 9100],
    ttl: 255,
    icmp: 8.9,
    name: "Front Office Printer",
    status: "known",
    notes: "Toner reordered automatically.",
  },
  {
    ip: "192.168.10.66",
    mac: "00:1B:D4:9A:3E:52",
    vendor: "Cisco Systems, Inc",
    hostname: "voip-desk-12",
    ports: [80, 5060],
    ttl: 255,
    icmp: 5.3,
  },
  {
    ip: "192.168.10.71",
    mac: "8C:77:12:B4:60:1C",
    vendor: "Samsung Electronics",
    hostname: null,
    ports: [8009, 8080],
    ttl: 64,
    icmp: 12.4,
  },
  {
    ip: "192.168.10.84",
    mac: "FC:65:DE:19:2D:6B",
    vendor: "Amazon Technologies",
    hostname: "conference-display",
    ports: [8008, 8009],
    ttl: 64,
    icmp: 9.8,
    name: "Conference Room Display",
  },
  {
    ip: "192.168.10.98",
    mac: "50:C7:BF:D1:44:03",
    vendor: "TP-LINK TECHNOLOGIES CO.,LTD.",
    hostname: "cam-loading-bay",
    ports: [80, 554],
    ttl: 64,
    icmp: 7.1,
    name: "Loading Bay Camera",
    status: "watched",
  },
  {
    ip: "192.168.10.120",
    mac: "80:2A:A8:33:9C:E7",
    vendor: "Ubiquiti Inc.",
    hostname: "ap-reception",
    ports: [22, 443],
    ttl: 64,
    icmp: 3.8,
    name: "Reception Access Point",
    status: "trusted",
  },
];

/** A tablet present in the earlier scans and missing from the newest one. */
const DEMO_MISSING: DemoDevice = {
  ip: "192.168.10.140",
  mac: "F0:18:98:5C:22:B1",
  vendor: "Apple, Inc.",
  hostname: "warehouse-tablet",
  ports: [62078],
  ttl: 64,
  icmp: 14.2,
  name: "Warehouse Tablet",
  status: "known",
};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

interface MockScanRow extends ScanSummary {
  hosts: HostResult[];
}

let nextScanId = 1;
let nextDeviceId = 1;
let nextEventScanId = 1000;
let cancelRequested = false;

const scans: MockScanRow[] = [];
const devices = new Map<number, Device>();
/** Normalized MAC to device id. */
const deviceByMac = new Map<string, number>();
/** Scan id to the observations it recorded, so history stays self-consistent. */
const observations = new Map<number, Array<{ deviceId: number; host: HostResult }>>();

function normalizeMac(mac: string): string | null {
  const hex = mac.replace(/[^0-9a-fA-F]/g, "").toUpperCase();
  if (hex.length !== 12) return null;
  const joined = hex.match(/.{2}/g)?.join(":") ?? "";
  if (joined === "FF:FF:FF:FF:FF:FF" || joined === "00:00:00:00:00:00") return null;
  return joined;
}

function osFromTtl(ttl: number): string | null {
  if (ttl >= 33 && ttl <= 64) return "Linux/Unix/macOS";
  if (ttl <= 128) return "Windows";
  return "Network device";
}

function hostFrom(device: DemoDevice, isoTime: string): HostResult {
  const hasPorts = device.ports.length > 0;
  return {
    ip: device.ip,
    hostname: device.hostname,
    mac: device.mac,
    vendor: device.vendor,
    open_ports: device.ports,
    response_ms: Math.max(1, Math.ceil(device.icmp)),
    icmp_ms: device.icmp,
    tcp_ms: hasPorts ? Math.round((device.icmp + 1.4) * 100) / 100 : null,
    ttl: device.ttl,
    os_guess: osFromTtl(device.ttl),
    last_seen: isoTime,
  };
}

function upsertDevice(
  host: HostResult,
  seenAt: string,
  seed?: DemoDevice,
): { id: number; existed: boolean } {
  const mac = host.mac ? normalizeMac(host.mac) : null;
  const key = mac
    ? `mac:${mac}`
    : host.hostname
      ? `hv:${host.hostname.toLowerCase()}|${(host.vendor ?? "").toLowerCase()}`
      : `ip:${host.ip}`;
  const existingId = mac ? deviceByMac.get(mac) : undefined;

  if (existingId != null) {
    const device = devices.get(existingId);
    if (device) {
      devices.set(existingId, {
        ...device,
        hostname: host.hostname ?? device.hostname,
        vendor: host.vendor ?? device.vendor,
        last_ip: host.ip,
        last_seen: seenAt,
        observation_count: device.observation_count + 1,
      });
    }
    return { id: existingId, existed: true };
  }

  const id = nextDeviceId++;
  devices.set(id, {
    id,
    identity_key: key,
    identity_source: mac ? "mac" : host.hostname ? "hostname-vendor" : "ip",
    mac,
    custom_name: seed?.name ?? null,
    hostname: host.hostname,
    vendor: host.vendor,
    last_ip: host.ip,
    first_seen: seenAt,
    last_seen: seenAt,
    status: seed?.status ?? "unclassified",
    notes: seed?.notes ?? null,
    observation_count: 1,
  });
  if (mac) deviceByMac.set(mac, id);
  return { id, existed: false };
}

// ---------------------------------------------------------------------------
// Comparison, mirroring the Rust rules
// ---------------------------------------------------------------------------

function identityOf(host: HostResult): string {
  const mac = host.mac ? normalizeMac(host.mac) : null;
  if (mac) return `mac:${mac}`;
  if (host.hostname?.trim()) {
    return `hv:${host.hostname.trim().toLowerCase()}|${(host.vendor ?? "").toLowerCase()}`;
  }
  return `ip:${host.ip}`;
}

function diffFields(before: HostResult, after: HostResult): FieldChange[] {
  const out: FieldChange[] = [];
  const plain = (field: string, label: string, from: string | null, to: string | null) => {
    const a = from?.trim() || null;
    const b = to?.trim() || null;
    if (a !== b) out.push({ field, label, from: a, to: b, added_ports: [], removed_ports: [] });
  };

  plain("ip", "IP address", before.ip, after.ip);
  plain("hostname", "Hostname", before.hostname, after.hostname);
  plain("vendor", "Manufacturer", before.vendor, after.vendor);
  plain("os_guess", "Operating system", before.os_guess, after.os_guess);

  const beforePorts = new Set(before.open_ports);
  const afterPorts = new Set(after.open_ports);
  const added = after.open_ports.filter((p) => !beforePorts.has(p));
  const removed = before.open_ports.filter((p) => !afterPorts.has(p));
  if (added.length > 0 || removed.length > 0) {
    out.push({
      field: "ports",
      label: "Open services",
      from: before.open_ports.map(serviceWithPort).join(", ") || "none",
      to: after.open_ports.map(serviceWithPort).join(", ") || "none",
      added_ports: added,
      removed_ports: removed,
    });
  }
  return out;
}

function nameFor(host: HostResult, deviceId: number | null): string {
  const custom = deviceId != null ? devices.get(deviceId)?.custom_name : null;
  if (custom?.trim()) return custom.trim();
  if (host.hostname?.trim()) return host.hostname.trim();
  if (host.vendor?.trim()) return `${host.vendor.trim()} (${host.ip})`;
  return host.ip;
}

function toDiff(
  kind: ChangeKind,
  host: HostResult,
  deviceId: number | null,
  fields: FieldChange[],
): DeviceDiff {
  return {
    kind,
    device_id: deviceId,
    name: nameFor(host, deviceId),
    ip: host.ip,
    mac: host.mac,
    vendor: host.vendor,
    hostname: host.hostname,
    last_seen: host.last_seen,
    fields,
  };
}

function compareScans(scanId: number, baselineId: number | null): ScanComparison {
  const baseline = baselineId != null ? scans.find((s) => s.id === baselineId) : undefined;
  if (!baseline) {
    return {
      scan_id: scanId,
      baseline_scan_id: null,
      baseline_created_at: null,
      baseline_target: null,
      reason:
        "No earlier scan of this target and profile exists, so there is nothing to compare against.",
      added: [],
      removed: [],
      changed: [],
    };
  }
  const current = scans.find((s) => s.id === scanId);
  if (!current) throw new Error(`Scan ${scanId} is no longer in the history.`);

  const deviceIdFor = (scan: number, ip: string): number | null =>
    observations.get(scan)?.find((o) => o.host.ip === ip)?.deviceId ?? null;

  const beforeByKey = new Map(baseline.hosts.map((h) => [identityOf(h), h]));
  const afterByKey = new Map(current.hosts.map((h) => [identityOf(h), h]));

  const added: DeviceDiff[] = [];
  const changed: DeviceDiff[] = [];
  const removed: DeviceDiff[] = [];

  for (const [key, host] of afterByKey) {
    const before = beforeByKey.get(key);
    const deviceId = deviceIdFor(scanId, host.ip);
    if (!before) {
      const device = deviceId != null ? devices.get(deviceId) : undefined;
      const seenBefore = device ? device.first_seen < baseline.created_at : false;
      added.push(toDiff(seenBefore ? "returned" : "new", host, deviceId, []));
      continue;
    }
    const fields = diffFields(before, host);
    if (fields.length > 0) changed.push(toDiff("changed", host, deviceId, fields));
  }
  for (const [key, host] of beforeByKey) {
    if (!afterByKey.has(key)) {
      removed.push(toDiff("missing", host, deviceIdFor(baseline.id, host.ip), []));
    }
  }

  return {
    scan_id: scanId,
    baseline_scan_id: baseline.id,
    baseline_created_at: baseline.created_at,
    baseline_target: baseline.target,
    reason: null,
    added,
    removed,
    changed,
  };
}

function recordScan(
  target: string,
  profile: string | null,
  hosts: HostResult[],
  createdAt: string,
  durationMs: number,
  seeds: Map<string, DemoDevice>,
  cancelled = false,
): MockScanRow {
  const id = nextScanId++;
  const baseline =
    [...scans].reverse().find((s) => s.target === target && s.profile === profile)?.id ?? null;

  observations.set(
    id,
    hosts.map((host) => ({
      deviceId: upsertDevice(host, createdAt, seeds.get(host.ip)).id,
      host,
    })),
  );

  const scan: MockScanRow = {
    id,
    target,
    target_key: `cidr:${target}`,
    profile,
    created_at: createdAt,
    duration_ms: durationMs,
    scanned: addressCount(target),
    probed: cancelled ? Math.round(addressCount(target) * 0.4) : addressCount(target),
    host_count: hosts.length,
    new_count: 0,
    missing_count: 0,
    changed_count: 0,
    status: cancelled ? "cancelled" : "completed",
    baseline_scan_id: baseline,
    hosts,
  };
  scans.push(scan);

  const comparison = compareScans(id, baseline);
  scan.new_count = comparison.added.filter((d) => d.kind === "new").length;
  scan.missing_count = comparison.removed.length;
  scan.changed_count = comparison.changed.length;
  return scan;
}

function byIp(a: HostResult, b: HostResult): number {
  const num = (ip: string) => ip.split(".").reduce((acc, o) => acc * 256 + Number(o), 0);
  return num(a.ip) - num(b.ip);
}

/**
 * Seed two earlier scans so the history and comparison views have real content
 * in the browser demo: a printer that moved and gained HTTPS, a display that
 * arrived, and a tablet that went missing.
 */
function seedHistory(): void {
  const seeds = new Map(DEMO_NETWORK.map((d) => [d.ip, d]));
  seeds.set(DEMO_MISSING.ip, DEMO_MISSING);

  const printer = DEMO_NETWORK.find((d) => d.hostname === "front-office-printer");
  if (!printer) return;
  const oldPrinter: DemoDevice = {
    ...printer,
    ip: "192.168.10.42",
    hostname: "printer-old",
    ports: [80, 515, 631, 9100],
  };
  seeds.set(oldPrinter.ip, { ...oldPrinter, name: "Front Office Printer" });

  const twoDaysAgo = new Date(Date.now() - 2 * 24 * 3600 * 1000).toISOString();
  const yesterday = new Date(Date.now() - 26 * 3600 * 1000).toISOString();

  for (const [stamp, duration] of [
    [twoDaysAgo, 18_400],
    [yesterday, 17_950],
  ] as const) {
    const hosts = DEMO_NETWORK.filter(
      (d) => d.hostname !== "front-office-printer" && d.hostname !== "conference-display",
    ).map((d) => hostFrom(d, stamp));
    hosts.push(hostFrom(oldPrinter, stamp), hostFrom(DEMO_MISSING, stamp));
    recordScan(DEMO_CIDR, "quick-lan", hosts.sort(byIp), stamp, duration, seeds);
  }
}

seedHistory();

// ---------------------------------------------------------------------------
// Mock API
// ---------------------------------------------------------------------------

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

export const mock = {
  async scan(opts: ScanOptions, listeners: ScanListeners): Promise<ScanResult> {
    cancelRequested = false;
    const scanId = nextEventScanId++;
    const preview = mock.previewScan(opts);
    const total = preview.total;
    const started = Date.now();

    listeners.onStarted?.({
      scan_id: scanId,
      target: opts.target,
      profile: opts.profile,
      total,
      port_count: preview.port_count,
      warning: preview.warning,
    });

    // Only the demo network has hosts. Any other target simply finds nothing,
    // which is the honest outcome for a mock with no network access.
    const target = opts.target.trim();
    const isDemo = target === DEMO_CIDR || target.startsWith("192.168.10.");
    const population = isDemo ? DEMO_NETWORK : [];
    const found: HostResult[] = [];
    const now = () => new Date().toISOString();

    // Walk the address space, emitting discoveries as they come up, at a pace
    // that lets the streaming table actually be seen working.
    const steps = 24;
    for (let step = 0; step < steps; step++) {
      if (cancelRequested) break;
      await sleep(90);
      const upTo = Math.round(((step + 1) / steps) * population.length);
      while (found.length < upTo) {
        const device = population[found.length];
        const enriched = hostFrom(device, now());
        // Discovery arrives with only what a probe can see. MAC, vendor and
        // hostname land in the update pass, exactly as the real scanner reports.
        listeners.onHostDiscovered?.({
          scan_id: scanId,
          host: { ...enriched, hostname: null, mac: null, vendor: null },
        });
        found.push(enriched);
      }
      listeners.onProgress?.({
        scan_id: scanId,
        done: Math.round(((step + 1) / steps) * total),
        total,
        found: found.length,
        phase: "probing",
        elapsed_ms: Date.now() - started,
      });
    }

    const cancelled = cancelRequested;
    const done = cancelled ? Math.round(total * 0.4) : total;
    listeners.onProgress?.({
      scan_id: scanId,
      done,
      total,
      found: found.length,
      phase: "resolving",
      elapsed_ms: Date.now() - started,
    });
    if (!cancelled) await sleep(300);

    for (const host of found) {
      listeners.onHostUpdated?.({ scan_id: scanId, host });
    }
    listeners.onProgress?.({
      scan_id: scanId,
      done,
      total,
      found: found.length,
      phase: cancelled ? "cancelled" : "done",
      elapsed_ms: Date.now() - started,
    });

    return {
      scan_id: scanId,
      target: opts.target,
      profile: opts.profile,
      duration_ms: Date.now() - started,
      scanned: total,
      probed: done,
      hosts: found,
      cancelled,
    };
  },

  cancelScan(): void {
    cancelRequested = true;
  },

  previewScan(opts: ScanOptions): ScanPreview {
    const total = addressCount(opts.target);
    const ports = opts.ports.length > 0 ? opts.ports.length : DEFAULT_PORTS.length;
    const workload = total * ports;
    return {
      total,
      port_count: ports,
      workload,
      warning:
        workload > 500_000
          ? `Large scan: ${workload.toLocaleString()} connection attempts across ${total.toLocaleString()} addresses. This can take a while and puts sustained load on your network.`
          : null,
    };
  },

  parsePortSpec(spec: string): number[] {
    const { ports, error } = parsePorts(spec);
    if (error) throw new Error(error);
    return ports.length > 0 ? ports : DEFAULT_PORTS;
  },

  serviceCatalog(): ServiceInfo[] {
    // Empty leaves the built-in fallback table in place.
    return [];
  },

  save(result: ScanResult): SavedScan {
    const seeds = new Map(DEMO_NETWORK.map((d) => [d.ip, d]));
    const scan = recordScan(
      result.target,
      result.profile,
      result.hosts,
      new Date().toISOString(),
      result.duration_ms,
      seeds,
      result.cancelled,
    );
    return { scan_id: scan.id, comparison: compareScans(scan.id, scan.baseline_scan_id) };
  },

  listScans(): ScanSummary[] {
    return scans.map(({ hosts: _hosts, ...summary }) => summary).sort((a, b) => b.id - a.id);
  },

  getScan(id: number): ScanDetail {
    const scan = scans.find((s) => s.id === id);
    if (!scan) throw new Error(`Scan ${id} is no longer in the history.`);
    const { hosts, ...summary } = scan;
    const rows = observations.get(id) ?? [];
    return {
      ...summary,
      hosts,
      devices: hosts.map((host) => {
        const deviceId = rows.find((r) => r.host.ip === host.ip)?.deviceId ?? null;
        const device = deviceId != null ? devices.get(deviceId) : undefined;
        return {
          ip: host.ip,
          device_id: deviceId,
          custom_name: device?.custom_name ?? null,
          status: device?.status ?? "unclassified",
          first_seen: device?.first_seen ?? null,
        };
      }),
    };
  },

  compareScan(id: number): ScanComparison {
    const scan = scans.find((s) => s.id === id);
    if (!scan) throw new Error(`Scan ${id} is no longer in the history.`);
    return compareScans(id, scan.baseline_scan_id);
  },

  deleteScan(id: number): void {
    const index = scans.findIndex((s) => s.id === id);
    if (index >= 0) scans.splice(index, 1);
    observations.delete(id);
  },

  pruneHistory(keep: number): number {
    const doomed = [...scans].sort((a, b) => b.id - a.id).slice(keep);
    for (const scan of doomed) mock.deleteScan(scan.id);
    return doomed.length;
  },

  listDevices(): Device[] {
    return [...devices.values()].sort((a, b) => b.last_seen.localeCompare(a.last_seen));
  },

  deviceDetail(id: number): DeviceDetail {
    const device = devices.get(id);
    if (!device) throw new Error(`Device ${id} is no longer in the inventory.`);

    const rows: DeviceObservation[] = [];
    for (const scan of [...scans].sort((a, b) => b.id - a.id)) {
      const hit = observations.get(scan.id)?.find((o) => o.deviceId === id);
      if (!hit) continue;
      rows.push({
        scan_id: scan.id,
        scan_target: scan.target,
        observed_at: hit.host.last_seen,
        ip: hit.host.ip,
        hostname: hit.host.hostname,
        vendor: hit.host.vendor,
        open_ports: hit.host.open_ports,
        response_ms: hit.host.response_ms,
        icmp_ms: hit.host.icmp_ms,
        tcp_ms: hit.host.tcp_ms,
        ttl: hit.host.ttl,
        os_guess: hit.host.os_guess,
      });
    }

    const previousIps: string[] = [];
    for (const row of rows) if (!previousIps.includes(row.ip)) previousIps.push(row.ip);

    const asHost = (o: DeviceObservation): HostResult => ({
      ip: o.ip,
      hostname: o.hostname,
      mac: null,
      vendor: o.vendor,
      open_ports: o.open_ports,
      response_ms: o.response_ms,
      icmp_ms: o.icmp_ms,
      tcp_ms: o.tcp_ms,
      ttl: o.ttl,
      os_guess: o.os_guess,
      last_seen: o.observed_at,
    });

    return {
      device,
      observations: rows,
      previous_ips: previousIps,
      recent_changes: rows.length >= 2 ? diffFields(asHost(rows[1]), asHost(rows[0])) : [],
    };
  },

  setDeviceName(id: number, name: string | null): void {
    const device = requireDevice(id);
    devices.set(id, { ...device, custom_name: name?.trim() || null });
  },

  setDeviceStatus(id: number, status: DeviceStatus): void {
    devices.set(id, { ...requireDevice(id), status });
  },

  setDeviceNotes(id: number, notes: string | null): void {
    const device = requireDevice(id);
    devices.set(id, { ...device, notes: notes?.trim() || null });
  },

  importDeviceLabels(labels: Record<string, string>): number {
    let adopted = 0;
    for (const [rawMac, label] of Object.entries(labels)) {
      const mac = normalizeMac(rawMac);
      const id = mac ? deviceByMac.get(mac) : undefined;
      if (id == null) continue;
      const device = devices.get(id);
      if (!device) continue;
      devices.set(id, {
        ...device,
        custom_name: device.custom_name ?? (label.trim() || null),
        status: device.status === "unclassified" ? "known" : device.status,
      });
      adopted += 1;
    }
    return adopted;
  },

  detectNetworks(): LocalNetwork[] {
    return [
      { interface: "Ethernet", ip: "192.168.10.27", prefix: 24, cidr: DEMO_CIDR, is_private: true },
    ];
  },
};

function requireDevice(id: number): Device {
  const device = devices.get(id);
  if (!device) throw new Error(`Device ${id} is no longer in the inventory.`);
  return device;
}

/** Address count for a target, used by the mock's scan preview. */
function addressCount(target: string): number {
  const t = target.trim();
  const cidr = t.match(/^(\d+\.\d+\.\d+\.\d+)\/(\d+)$/);
  if (cidr) {
    const bits = Number(cidr[2]);
    if (bits >= 31) return 2 ** (32 - bits);
    return Math.max(1, 2 ** (32 - bits) - 2);
  }
  const range = t.match(/^\d+\.\d+\.\d+\.(\d+)\s*-\s*(?:\d+\.\d+\.\d+\.)?(\d+)$/);
  if (range) return Math.abs(Number(range[2]) - Number(range[1])) + 1;
  return 1;
}
