// Pure-TypeScript mock backend.
//
// Runs entirely in the browser with no Rust build, so the whole interface is
// developable, reviewable and screenshotable without installing a toolchain. The
// API layer falls back to it automatically when the app is not inside Tauri.
//
// It mirrors the real backend's behaviour rather than returning placeholders: it
// streams host events with the same shapes and ordering, resolves device identity
// by MAC, keeps two networks apart by their gateways, applies the same presence
// rules, and records change events under the same deterministic keys. Every value
// is deliberately fictional, so a screenshot can never expose a real network. The
// native app never uses it.

import type {
  BulkOutcome,
  ChangeEvent,
  ChangeFeed,
  ChangeState,
  ChangeType,
  Device,
  DeviceDetail,
  DeviceDiff,
  DeviceObservation,
  DeviceStatus,
  FieldChange,
  HostResult,
  InventoryRow,
  InventorySummary,
  LocalNetwork,
  NetworkOption,
  NetworkScope,
  PresenceState,
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
import { PUBLIC_IP_PROVIDERS, abortError, lookupPublicIp } from "./publicIp";

/** The network the demo's live scans run against. */
const DEMO_CIDR = "192.168.1.0/24";
const OFFICE_CIDR = "10.20.0.0/24";

/** Coverage keys, mirroring the backend's ports-and-discovery-mode signature. */
const QUICK_COVERAGE = "v1|arp:auto|ports:22,80,443,445,631,554,3389,5000,8009,9100";
const WIDE_COVERAGE = "v1|arp:auto|ports:1-1024";

/** Mirrors the backend's partial-scan comparison reason. */
export const PARTIAL_SCAN_REASON =
  "This scan was stopped before every address was checked, so missing devices and complete network changes cannot be determined reliably.";

/**
 * The two networks the demo knows about.
 *
 * Network separation only means anything when there is more than one network to
 * keep apart, so the demo carries a home network and an office one, each with
 * its own gateway, its own devices and its own history.
 */
const DEMO_SCOPES: NetworkScope[] = [
  {
    id: 1,
    stable_key: `target:cidr:${DEMO_CIDR}|gw:F4:92:BF:1A:0C:31`,
    display_name: "Home Wi-Fi",
    canonical_target: `cidr:${DEMO_CIDR}`,
    gateway_mac: "F4:92:BF:1A:0C:31",
    interface_hint: "Wi-Fi",
    created_at: new Date(Date.now() - 40 * 24 * 3600 * 1000).toISOString(),
    updated_at: new Date().toISOString(),
    device_count: 0,
    scan_count: 0,
  },
  {
    id: 2,
    stable_key: `target:cidr:${OFFICE_CIDR}|gw:B4:FB:E4:7C:22:90`,
    display_name: "Office",
    canonical_target: `cidr:${OFFICE_CIDR}`,
    gateway_mac: "B4:FB:E4:7C:22:90",
    interface_hint: "Ethernet",
    created_at: new Date(Date.now() - 25 * 24 * 3600 * 1000).toISOString(),
    updated_at: new Date(Date.now() - 3 * 24 * 3600 * 1000).toISOString(),
    device_count: 0,
    scan_count: 0,
  },
];

/** One device on a fictional demo network. */
interface DemoDevice {
  ip: string;
  mac: string | null;
  vendor: string | null;
  hostname: string | null;
  ports: number[];
  ttl: number;
  icmp: number;
  name?: string;
  status?: DeviceStatus;
  notes?: string;
}

/**
 * The home network as the latest scan sees it: a router, a laptop, a desktop, a
 * phone, a printer, a TV, a NAS, a camera, a games console and one device
 * nothing could identify. Deliberately imperfect — two of them resolve no
 * hostname at all, and one has no manufacturer either — because a demo where
 * every device is neatly identified would be a lie about what scanning is like.
 */
const HOME_DEVICES: DemoDevice[] = [
  {
    ip: "192.168.1.1",
    mac: "F4:92:BF:1A:0C:31",
    vendor: "Ubiquiti Inc.",
    hostname: "gateway.home",
    ports: [53, 80, 443],
    ttl: 64,
    icmp: 0.9,
    name: "Home Router",
    status: "trusted",
    notes: "Fibre router. Firmware checked when the ISP prompts.",
  },
  {
    ip: "192.168.1.12",
    mac: "3C:22:FB:88:41:D0",
    vendor: "Apple, Inc.",
    hostname: "macbook-air",
    ports: [22],
    ttl: 64,
    icmp: 2.4,
    status: "trusted",
  },
  {
    ip: "192.168.1.15",
    mac: "18:66:DA:70:2B:14",
    vendor: "Dell Inc.",
    hostname: "desktop-study",
    ports: [135, 139, 445, 3389],
    ttl: 128,
    icmp: 1.1,
    name: "Study Desktop",
    status: "known",
  },
  {
    ip: "192.168.1.23",
    mac: "8C:77:12:B4:60:1C",
    vendor: "Samsung Electronics",
    hostname: null,
    ports: [],
    ttl: 64,
    icmp: 12.4,
  },
  {
    ip: "192.168.1.31",
    mac: "3C:D9:2B:6F:08:AA",
    vendor: "Hewlett Packard",
    hostname: "office-printer",
    ports: [80, 443, 515, 631, 9100],
    ttl: 255,
    icmp: 8.9,
    name: "Office Printer",
    status: "known",
    notes: "Toner reordered automatically. Web interface on HTTPS since the firmware update.",
  },
  {
    ip: "192.168.1.44",
    mac: "FC:65:DE:19:2D:6B",
    vendor: "Samsung Electronics",
    hostname: "livingroom-tv",
    ports: [8001, 8009],
    ttl: 64,
    icmp: 9.8,
    name: "Living Room TV",
  },
  {
    ip: "192.168.1.50",
    mac: "00:11:32:5D:A2:77",
    vendor: "Synology Incorporated",
    hostname: "nas-home",
    ports: [22, 443, 445, 5000],
    ttl: 64,
    icmp: 1.2,
    name: "Home NAS",
    status: "trusted",
    notes: "Nightly backup target. Only SMB and the web console should be open.",
  },
  {
    ip: "192.168.1.60",
    mac: "50:C7:BF:D1:44:03",
    vendor: "TP-LINK TECHNOLOGIES CO.,LTD.",
    hostname: "cam-driveway",
    ports: [80, 554, 8080],
    ttl: 64,
    icmp: 7.1,
    name: "Driveway Camera",
    status: "ignored",
    notes: "Firmware reshuffles its ports on every reboot, so its changes are not worth reviewing.",
  },
  {
    ip: "192.168.1.72",
    mac: "78:C8:81:4A:33:9E",
    vendor: "Sony Interactive Entertainment",
    hostname: null,
    ports: [],
    ttl: 64,
    icmp: 15.2,
    name: "Games Console",
  },
  {
    // Nothing resolved: no hostname, and the OUI prefix is not in the table.
    ip: "192.168.1.88",
    mac: "6A:11:9C:03:D7:41",
    vendor: null,
    hostname: null,
    ports: [8888],
    ttl: 64,
    icmp: 22.6,
  },
];

/** In the first home scan only, so it reads as missing from the latest one. */
const HOME_TABLET: DemoDevice = {
  ip: "192.168.1.105",
  mac: "F0:18:98:5C:22:B1",
  vendor: "Apple, Inc.",
  hostname: "kitchen-ipad",
  ports: [62078],
  ttl: 64,
  icmp: 14.2,
  name: "Kitchen Tablet",
  status: "known",
};

/** The office network, which the demo never scans live: it is history only. */
const OFFICE_DEVICES: DemoDevice[] = [
  {
    ip: "10.20.0.1",
    mac: "B4:FB:E4:7C:22:90",
    vendor: "Ubiquiti Inc.",
    hostname: "office-gw",
    ports: [53, 80, 443],
    ttl: 64,
    icmp: 0.7,
    name: "Office Gateway",
    status: "trusted",
  },
  {
    ip: "10.20.0.10",
    mac: "00:15:5D:3C:11:9E",
    vendor: "Microsoft Corporation",
    hostname: "srv-files01",
    ports: [135, 139, 445, 3389],
    ttl: 128,
    icmp: 1.0,
    name: "File Server",
    status: "known",
    notes: "Shares the accounts folder. RDP is deliberate.",
  },
  {
    ip: "10.20.0.14",
    mac: "54:BF:64:2B:91:0C",
    vendor: "Lenovo",
    hostname: "ws-reception",
    ports: [135, 445],
    ttl: 128,
    icmp: 2.9,
  },
  {
    ip: "10.20.0.22",
    mac: "00:1B:A9:44:60:F7",
    vendor: "Brother Industries, Ltd.",
    hostname: "printer-back-office",
    ports: [80, 515, 9100],
    ttl: 255,
    icmp: 6.4,
    name: "Back Office Printer",
  },
  {
    ip: "10.20.0.30",
    mac: "24:5E:BE:07:3C:12",
    vendor: "QNAP Systems, Inc.",
    hostname: "nas-office",
    ports: [22, 443, 445],
    ttl: 64,
    icmp: 1.4,
    name: "Office NAS",
    status: "trusted",
  },
  {
    ip: "10.20.0.41",
    mac: "00:1B:D4:9A:3E:52",
    vendor: "Cisco Systems, Inc",
    hostname: "voip-desk-12",
    ports: [80, 5060],
    ttl: 255,
    icmp: 5.3,
  },
  {
    ip: "10.20.0.55",
    mac: "BC:AD:28:66:1F:90",
    vendor: "Hangzhou Hikvision",
    hostname: "cam-lobby",
    ports: [80, 554],
    ttl: 64,
    icmp: 4.8,
    name: "Lobby Camera",
  },
];

/**
 * Seen once by a wider scan and never by one with the coverage the office's
 * latest scan used. Its absence proves nothing, so it stays Unknown, which is
 * the state that is hardest to demonstrate and easiest to get wrong.
 */
const OFFICE_ONE_OFF: DemoDevice = {
  ip: "10.20.0.70",
  mac: "A4:5E:60:D1:07:22",
  vendor: null,
  hostname: null,
  ports: [9001],
  ttl: 64,
  icmp: 31.4,
};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

interface MockScanRow extends ScanSummary {
  hosts: HostResult[];
}

let nextScanId = 1;
let nextDeviceId = 1;
let nextEventId = 1;
let nextEventScanId = 1000;
let cancelRequested = false;

const scans: MockScanRow[] = [];
const devices = new Map<number, Device>();
/** Scope id and normalized MAC to device id: identity never crosses a network. */
const deviceByMac = new Map<string, number>();
/** Scan id to the observations it recorded, so history stays self-consistent. */
const observations = new Map<number, Array<{ deviceId: number; host: HostResult }>>();
/** Persisted change events, oldest first. */
const changeEvents: ChangeEvent[] = [];

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
  scopeId: number,
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
  // Scoped, so the same MAC on two networks is two devices.
  const macKey = mac ? `${scopeId}|${mac}` : null;
  const existingId = macKey ? deviceByMac.get(macKey) : undefined;

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
    network_scope_id: scopeId,
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
  if (macKey) deviceByMac.set(macKey, id);
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
  kind: DeviceDiff["kind"],
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

function emptyComparison(scanId: number, reason: string): ScanComparison {
  return {
    scan_id: scanId,
    baseline_scan_id: null,
    baseline_created_at: null,
    baseline_target: null,
    reason,
    added: [],
    removed: [],
    changed: [],
  };
}

function compareScans(scanId: number, baselineId: number | null): ScanComparison {
  const current = scans.find((s) => s.id === scanId);
  if (!current) throw new Error(`Scan ${scanId} is no longer in the history.`);
  // A cancelled scan did not observe its whole target: no comparison, ever.
  if (current.status === "cancelled") {
    return emptyComparison(scanId, PARTIAL_SCAN_REASON);
  }
  const baseline = baselineId != null ? scans.find((s) => s.id === baselineId) : undefined;
  if (!baseline) {
    return emptyComparison(
      scanId,
      "No earlier completed scan with this target and coverage exists, so there is nothing to compare against.",
    );
  }

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

// ---------------------------------------------------------------------------
// Change events, mirroring the Rust rules
// ---------------------------------------------------------------------------

const FIELD_TO_CHANGE: Record<string, ChangeType> = {
  ip: "ip_changed",
  hostname: "hostname_changed",
  vendor: "vendor_changed",
  os_guess: "os_changed",
  mac: "mac_changed",
  ports: "ports_changed",
};

/**
 * Record a completed scan's changes.
 *
 * The event key is the scan, the device and the kind of change, exactly as the
 * backend builds it, so re-recording the same comparison writes nothing. A
 * device the operator ignored gets its events written already-ignored, so they
 * stay out of the default inbox without being lost.
 */
function recordChangeEvents(scan: MockScanRow, comparison: ScanComparison): void {
  const baseline = scans.find((s) => s.id === comparison.baseline_scan_id);
  if (!baseline) return;

  const write = (
    diff: DeviceDiff,
    changeType: ChangeType,
    oldValue: string | null,
    newValue: string | null,
    opened: number[],
    closed: number[],
  ) => {
    const subject = diff.device_id != null ? `d${diff.device_id}` : `ip:${diff.ip}`;
    const key = `s${scan.id}|${subject}|${changeType}`;
    if (changeEvents.some((e) => e.event_key === key)) return;
    const device = diff.device_id != null ? devices.get(diff.device_id) : undefined;
    changeEvents.push({
      id: nextEventId++,
      event_key: key,
      scan_id: scan.id,
      baseline_scan_id: baseline.id,
      network_scope_id: scan.network_scope_id,
      network_name: scan.scope_name,
      device_id: diff.device_id,
      device_label: diff.name,
      ip: diff.ip,
      mac: diff.mac,
      vendor: diff.vendor,
      change_type: changeType,
      old_value: oldValue,
      new_value: newValue,
      opened_ports: opened,
      closed_ports: closed,
      state: device?.status === "ignored" ? "ignored" : "unreviewed",
      created_at: scan.created_at,
      scan_at: scan.created_at,
      baseline_at: baseline.created_at,
      acknowledged_at: null,
      device_status: device?.status ?? null,
    });
  };

  for (const diff of comparison.added) {
    write(diff, diff.kind === "returned" ? "device_returned" : "device_added", null, diff.ip, [], []);
  }
  for (const diff of comparison.removed) {
    write(diff, "device_missing", diff.ip, null, [], []);
  }
  for (const diff of comparison.changed) {
    for (const field of diff.fields) {
      const changeType = FIELD_TO_CHANGE[field.field];
      if (!changeType) continue;
      write(diff, changeType, field.from, field.to, field.added_ports, field.removed_ports);
    }
  }
}

function recordScan(options: {
  scopeId: number;
  target: string;
  profile: string | null;
  coverageKey: string;
  hosts: HostResult[];
  createdAt: string;
  durationMs: number;
  seeds: Map<string, DemoDevice>;
  cancelled?: boolean;
}): MockScanRow {
  const { scopeId, target, profile, coverageKey, hosts, createdAt, seeds } = options;
  const cancelled = options.cancelled ?? false;
  const id = nextScanId++;
  // Mirrors the backend baseline rule: only a *completed* scan in the same scope
  // with matching target and coverage can anchor a comparison, and a cancelled
  // scan gets none.
  const baseline = cancelled
    ? null
    : ([...scans]
        .reverse()
        .find(
          (s) =>
            s.network_scope_id === scopeId &&
            s.target_key === `cidr:${target}` &&
            s.coverage_key === coverageKey &&
            s.status === "completed",
        )?.id ?? null);

  observations.set(
    id,
    hosts.map((host) => ({
      deviceId: upsertDevice(scopeId, host, createdAt, seeds.get(host.ip)).id,
      host,
    })),
  );

  const scope = DEMO_SCOPES.find((s) => s.id === scopeId);
  const scan: MockScanRow = {
    id,
    target,
    target_key: `cidr:${target}`,
    profile,
    created_at: createdAt,
    duration_ms: options.durationMs,
    scanned: addressCount(target),
    probed: cancelled ? Math.round(addressCount(target) * 0.4) : addressCount(target),
    host_count: hosts.length,
    new_count: 0,
    missing_count: 0,
    changed_count: 0,
    status: cancelled ? "cancelled" : "completed",
    baseline_scan_id: baseline,
    network_scope_id: scopeId,
    scope_name: scope?.display_name ?? null,
    coverage_key: coverageKey,
    hosts,
  };
  scans.push(scan);

  const comparison = compareScans(id, baseline);
  scan.new_count = comparison.added.filter((d) => d.kind === "new").length;
  scan.missing_count = comparison.removed.length;
  scan.changed_count = comparison.changed.length;
  // A cancelled scan has no baseline and therefore no events, which is the
  // partial-scan rule falling out of the same code path the backend uses.
  if (baseline != null) recordChangeEvents(scan, comparison);
  return scan;
}

function byIp(a: HostResult, b: HostResult): number {
  const num = (ip: string) => ip.split(".").reduce((acc, o) => acc * 256 + Number(o), 0);
  return num(a.ip) - num(b.ip);
}

function daysAgo(days: number, hour = 9): string {
  const d = new Date(Date.now() - days * 24 * 3600 * 1000);
  d.setHours(hour, 12, 0, 0);
  return d.toISOString();
}

/**
 * Seed both networks with enough history that the Inventory, the Changes inbox
 * and the comparison view all have real content in the browser demo.
 *
 * Home Wi-Fi gets two completed scans: the printer moves address and gains
 * HTTPS, the laptop's hostname changes, the TV arrives and the kitchen tablet
 * stops answering. The camera the operator ignored also changes, which is what
 * shows that ignoring keeps the record without filling the inbox.
 *
 * The office gets a wide scan first and two narrower ones afterwards, so one
 * device is only ever seen under different coverage and honestly reads Unknown.
 */
function seedHistory(): void {
  const homeSeeds = new Map(HOME_DEVICES.map((d) => [d.ip, d]));
  homeSeeds.set(HOME_TABLET.ip, HOME_TABLET);

  const printer = HOME_DEVICES.find((d) => d.hostname === "office-printer")!;
  const oldPrinter: DemoDevice = { ...printer, ip: "192.168.1.28", ports: [80, 515, 631, 9100] };
  homeSeeds.set(oldPrinter.ip, oldPrinter);
  const laptop = HOME_DEVICES.find((d) => d.hostname === "macbook-air")!;
  const oldLaptop: DemoDevice = { ...laptop, hostname: "macbook" };
  const camera = HOME_DEVICES.find((d) => d.hostname === "cam-driveway")!;
  const quietCamera: DemoDevice = { ...camera, ports: [80, 554] };

  const officeSeeds = new Map(OFFICE_DEVICES.map((d) => [d.ip, d]));
  officeSeeds.set(OFFICE_ONE_OFF.ip, OFFICE_ONE_OFF);
  const officePrinter = OFFICE_DEVICES.find((d) => d.hostname === "printer-back-office")!;
  const oldOfficePrinter: DemoDevice = { ...officePrinter, ip: "10.20.0.19" };
  officeSeeds.set(oldOfficePrinter.ip, oldOfficePrinter);
  const officeNas = OFFICE_DEVICES.find((d) => d.hostname === "nas-office")!;
  const oldOfficeNas: DemoDevice = { ...officeNas, ports: [22, 445] };

  // Oldest first, so scan ids ascend with time and History reads in order.
  const timeline = [
    // A wide office sweep. Its coverage differs from everything after it, which
    // is what leaves the one-off device honestly Unknown rather than Missing.
    {
      scopeId: 2,
      target: OFFICE_CIDR,
      profile: "full-tcp",
      coverageKey: WIDE_COVERAGE,
      createdAt: daysAgo(21),
      durationMs: 184_000,
      seeds: officeSeeds,
      devices: [...OFFICE_DEVICES, OFFICE_ONE_OFF],
    },
    {
      scopeId: 2,
      target: OFFICE_CIDR,
      profile: "quick-lan",
      coverageKey: QUICK_COVERAGE,
      createdAt: daysAgo(10),
      durationMs: 16_200,
      seeds: officeSeeds,
      devices: OFFICE_DEVICES.filter(
        (d) => d.hostname !== "printer-back-office" && d.hostname !== "nas-office",
      ).concat([oldOfficePrinter, oldOfficeNas]),
    },
    // The home network before the printer moved, with the tablet still present
    // and no TV yet.
    {
      scopeId: 1,
      target: DEMO_CIDR,
      profile: "quick-lan",
      coverageKey: QUICK_COVERAGE,
      createdAt: daysAgo(6),
      durationMs: 18_400,
      seeds: homeSeeds,
      devices: HOME_DEVICES.filter(
        (d) =>
          d.hostname !== "office-printer" &&
          d.hostname !== "livingroom-tv" &&
          d.hostname !== "macbook-air" &&
          d.hostname !== "cam-driveway",
      ).concat([oldPrinter, oldLaptop, quietCamera, HOME_TABLET]),
    },
    // The office printer moves and the NAS opens HTTPS.
    {
      scopeId: 2,
      target: OFFICE_CIDR,
      profile: "quick-lan",
      coverageKey: QUICK_COVERAGE,
      createdAt: daysAgo(3),
      durationMs: 16_050,
      seeds: officeSeeds,
      devices: OFFICE_DEVICES,
    },
    // The home printer moves and gains HTTPS, the laptop is renamed, the TV
    // arrives and the tablet stops answering.
    {
      scopeId: 1,
      target: DEMO_CIDR,
      profile: "quick-lan",
      coverageKey: QUICK_COVERAGE,
      createdAt: daysAgo(1, 20),
      durationMs: 17_950,
      seeds: homeSeeds,
      devices: HOME_DEVICES,
    },
  ];

  for (const entry of timeline) {
    recordScan({
      scopeId: entry.scopeId,
      target: entry.target,
      profile: entry.profile,
      coverageKey: entry.coverageKey,
      hosts: entry.devices.map((d) => hostFrom(d, entry.createdAt)).sort(byIp),
      createdAt: entry.createdAt,
      durationMs: entry.durationMs,
      seeds: entry.seeds,
    });
  }
}

/**
 * The browser tests need to see the empty states as well as the populated ones,
 * and a demo that is empty until you scan is exactly what a new install looks
 * like. `?demo=empty` skips the seeding; anything else gets the full history.
 * This affects the browser demo only — the native app never loads this module.
 */
function seedingWanted(): boolean {
  if (typeof window === "undefined") return true;
  try {
    return new URLSearchParams(window.location.search).get("demo") !== "empty";
  } catch {
    return true;
  }
}

if (seedingWanted()) seedHistory();

// ---------------------------------------------------------------------------
// Presence, mirroring the Rust rules
// ---------------------------------------------------------------------------

/** A scope's most recent scan that both completed and recorded its coverage. */
function referenceScan(scopeId: number | null): MockScanRow | null {
  if (scopeId == null) return null;
  const candidates = scans.filter(
    (s) =>
      s.network_scope_id === scopeId &&
      s.status === "completed" &&
      s.coverage_key !== "" &&
      !s.coverage_key.startsWith("legacy:"),
  );
  return candidates.length > 0 ? candidates[candidates.length - 1] : null;
}

function presenceOf(deviceId: number, scopeId: number | null): PresenceState {
  const reference = referenceScan(scopeId);
  if (!reference) return "unknown";
  const seenByReference = observations
    .get(reference.id)
    ?.some((o) => o.deviceId === deviceId);
  if (seenByReference) return "present";
  // Absence only means something if a scan with the same coverage saw it before.
  const comparable = scans.some(
    (s) =>
      s.network_scope_id === scopeId &&
      s.status === "completed" &&
      s.target_key === reference.target_key &&
      s.coverage_key === reference.coverage_key &&
      observations.get(s.id)?.some((o) => o.deviceId === deviceId),
  );
  return comparable ? "missing" : "unknown";
}

function observationsFor(deviceId: number): Array<{ scan: MockScanRow; host: HostResult }> {
  const out: Array<{ scan: MockScanRow; host: HostResult }> = [];
  for (const scan of [...scans].sort((a, b) => b.id - a.id)) {
    const hit = observations.get(scan.id)?.find((o) => o.deviceId === deviceId);
    if (hit) out.push({ scan, host: hit.host });
  }
  return out;
}

function inventoryRowFor(device: Device): InventoryRow {
  const sightings = observationsFor(device.id);
  const latest = sightings[0]?.host;
  const currentIp = latest?.ip ?? device.last_ip;
  const previousIps: string[] = [];
  for (const { host } of sightings) {
    if (host.ip !== currentIp && !previousIps.includes(host.ip)) previousIps.push(host.ip);
  }
  const reference = referenceScan(device.network_scope_id);
  const scope = DEMO_SCOPES.find((s) => s.id === device.network_scope_id);
  return {
    device_id: device.id,
    network_scope_id: device.network_scope_id,
    network_name: scope?.display_name ?? null,
    identity_source: device.identity_source,
    display_name: nameFor(
      latest ?? {
        ip: currentIp ?? "",
        hostname: device.hostname,
        mac: device.mac,
        vendor: device.vendor,
        open_ports: [],
        response_ms: null,
        icmp_ms: null,
        tcp_ms: null,
        ttl: null,
        os_guess: null,
        last_seen: device.last_seen,
      },
      device.id,
    ),
    custom_name: device.custom_name,
    hostname: device.hostname,
    current_ip: currentIp,
    previous_ips: previousIps.slice(0, 8),
    mac: device.mac,
    vendor: device.vendor,
    os_guess: latest?.os_guess ?? null,
    status: device.status,
    presence: presenceOf(device.id, device.network_scope_id),
    first_seen: device.first_seen,
    last_seen: device.last_seen,
    last_completed_scan_id: reference?.id ?? null,
    last_completed_scan_at: reference?.created_at ?? null,
    observation_count: sightings.length,
    open_ports: latest?.open_ports ?? [],
    notes_present: Boolean(device.notes?.trim()),
    notes_excerpt: device.notes?.slice(0, 160) ?? null,
    latest_response_ms: latest?.response_ms ?? null,
    latest_icmp_ms: latest?.icmp_ms ?? null,
    latest_tcp_ms: latest?.tcp_ms ?? null,
  };
}

/** Resolve an event's label and status against the device as it is now. */
function refreshedEvent(event: ChangeEvent): ChangeEvent {
  const device = event.device_id != null ? devices.get(event.device_id) : undefined;
  if (!device) return event;
  const scope = DEMO_SCOPES.find((s) => s.id === event.network_scope_id);
  return {
    ...event,
    device_label: nameFor(
      {
        ip: event.ip ?? "",
        hostname: device.hostname,
        mac: device.mac,
        vendor: device.vendor,
        open_ports: [],
        response_ms: null,
        icmp_ms: null,
        tcp_ms: null,
        ttl: null,
        os_guess: null,
        last_seen: device.last_seen,
      },
      device.id,
    ),
    network_name: scope?.display_name ?? event.network_name,
    device_status: device.status,
  };
}

// ---------------------------------------------------------------------------
// Mock API
// ---------------------------------------------------------------------------

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

// --- Public IP --------------------------------------------------------------
//
// The demo answers the lookup from scripted providers rather than the real
// ones, for two reasons: the browser demo must never make an outbound request
// on a visitor's behalf, and a screenshot must never contain a real public
// address. The scripted providers are driven through the same
// `lookupPublicIp` used in the packaged app, so the fallback order, the
// response validation and the error normalisation under test are the real
// ones rather than a second implementation of them.

/** Reserved for documentation by RFC 5737, so it can never be anyone's address. */
const DEMO_PUBLIC_IP = "203.0.113.24";
/** A second documentation address, returned only by the fallback provider. */
const DEMO_FALLBACK_PUBLIC_IP = "198.51.100.17";

/** Selected with `?publicip=` in the browser demo. */
export type PublicIpScenario = "ok" | "fallback" | "fail" | "flaky" | "slow";

const PUBLIC_IP_SCENARIOS: PublicIpScenario[] = ["ok", "fallback", "fail", "flaky", "slow"];

function publicIpScenario(): PublicIpScenario {
  if (typeof window === "undefined") return "ok";
  const value = new URLSearchParams(window.location.search).get("publicip");
  return PUBLIC_IP_SCENARIOS.includes(value as PublicIpScenario)
    ? (value as PublicIpScenario)
    : "ok";
}

/** Attempts so far, so `flaky` can fail the first lookup and pass the retry. */
let publicIpAttempts = 0;

function abortableSleep(ms: number, signal?: AbortSignal | null): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(abortError());
      return;
    }
    const onAbort = () => {
      clearTimeout(timer);
      reject(abortError());
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

/**
 * A `fetch` that answers only the two provider URLs, according to `scenario`.
 *
 * Delays are long enough that the loading state is a state a person can
 * actually see and a test can actually assert on, and short enough that the
 * suite is not slowed down by them.
 */
function scriptedProviderFetch(scenario: PublicIpScenario, attempt: number): typeof fetch {
  const primaryUrl = PUBLIC_IP_PROVIDERS[0].url;

  return async (input, init) => {
    const url =
      typeof input === "string" ? input : input instanceof URL ? input.href : (input as Request).url;
    const primary = url === primaryUrl;

    await abortableSleep(scenario === "slow" ? 3_400 : 420, init?.signal);

    // A provider the demo cannot reach fails exactly as a browser would.
    const unreachable = () => {
      throw new TypeError("Failed to fetch");
    };

    switch (scenario) {
      case "fail":
        return unreachable();
      case "flaky":
        if (attempt <= 1) return unreachable();
        break;
      case "fallback":
        // The first provider is up but broken, which is the case the fallback
        // exists for and the one that never happens on demand.
        if (primary) return new Response("upstream error", { status: 503 });
        return new Response(`${DEMO_FALLBACK_PUBLIC_IP}\n`, {
          status: 200,
          headers: { "content-type": "text/plain" },
        });
      default:
        break;
    }

    if (!primary) {
      return new Response(`${DEMO_FALLBACK_PUBLIC_IP}\n`, {
        status: 200,
        headers: { "content-type": "text/plain" },
      });
    }
    return new Response(JSON.stringify({ ip: DEMO_PUBLIC_IP }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };
}

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

    // Only the home network has hosts. Any other target simply finds nothing,
    // which is the honest outcome for a mock with no network access.
    const target = opts.target.trim();
    const isDemo = target === DEMO_CIDR || target.startsWith("192.168.1.");
    const population = isDemo ? HOME_DEVICES : [];
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
      ports: opts.ports.length > 0 ? opts.ports : DEFAULT_PORTS,
      arp_assist: opts.arp_assist,
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
    const seeds = new Map(HOME_DEVICES.map((d) => [d.ip, d]));
    const scan = recordScan({
      scopeId: 1,
      target: result.target,
      profile: result.profile,
      coverageKey: QUICK_COVERAGE,
      hosts: result.hosts,
      createdAt: new Date().toISOString(),
      durationMs: result.duration_ms,
      seeds,
      cancelled: result.cancelled,
    });
    return { scan_id: scan.id, comparison: compareScans(scan.id, scan.baseline_scan_id) };
  },

  listScans(): ScanSummary[] {
    return scans
      .map(({ hosts: _hosts, ...summary }) => withScopeName(summary))
      .sort((a, b) => b.id - a.id);
  },

  getScan(id: number): ScanDetail {
    const scan = scans.find((s) => s.id === id);
    if (!scan) throw new Error(`Scan ${id} is no longer in the history.`);
    const { hosts, ...rest } = scan;
    const summary = withScopeName(rest);
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
    // Change events outlive the scan that produced them, exactly as they do in
    // the database, so a pruned history does not erase what was reviewed.
    for (const event of changeEvents) {
      if (event.scan_id === id) event.scan_id = null;
    }
  },

  pruneHistory(keep: number): number {
    const doomed = [...scans].sort((a, b) => b.id - a.id).slice(keep);
    for (const scan of doomed) mock.deleteScan(scan.id);
    return doomed.length;
  },

  listDevices(): Device[] {
    return [...devices.values()].sort((a, b) => b.last_seen.localeCompare(a.last_seen));
  },

  inventory(): InventorySummary {
    const rows = [...devices.values()]
      .map(inventoryRowFor)
      .sort((a, b) => b.last_seen.localeCompare(a.last_seen) || b.device_id - a.device_id);

    const networks = new Map<number, NetworkOption>();
    for (const row of rows) {
      if (row.network_scope_id == null) continue;
      const existing = networks.get(row.network_scope_id);
      if (existing) existing.device_count += 1;
      else {
        networks.set(row.network_scope_id, {
          id: row.network_scope_id,
          name: row.network_name ?? "Network",
          device_count: 1,
        });
      }
    }

    return {
      rows,
      networks: [...networks.values()].sort((a, b) =>
        a.name.toLowerCase().localeCompare(b.name.toLowerCase()),
      ),
      present: rows.filter((r) => r.presence === "present").length,
      missing: rows.filter((r) => r.presence === "missing").length,
      unknown: rows.filter((r) => r.presence === "unknown").length,
      needs_completed_scan:
        rows.length > 0 && rows.every((r) => r.last_completed_scan_id == null),
    };
  },

  changeEvents(): ChangeFeed {
    const events = [...changeEvents].reverse().map(refreshedEvent);
    return {
      events,
      unreviewed: events.filter((e) => e.state === "unreviewed").length,
      total: events.length,
      truncated: false,
      starts_after_scan_id: 0,
    };
  },

  setChangeState(ids: number[], state: ChangeState): BulkOutcome {
    const missing: number[] = [];
    let updated = 0;
    for (const id of ids) {
      const event = changeEvents.find((e) => e.id === id);
      if (!event) {
        missing.push(id);
        continue;
      }
      event.state = state;
      event.acknowledged_at = state === "acknowledged" ? new Date().toISOString() : null;
      updated += 1;
    }
    return { updated, missing };
  },

  setDeviceStatuses(ids: number[], status: DeviceStatus): BulkOutcome {
    const missing: number[] = [];
    let updated = 0;
    for (const id of ids) {
      const device = devices.get(id);
      if (!device) {
        missing.push(id);
        continue;
      }
      devices.set(id, { ...device, status });
      updated += 1;
      if (status === "ignored") {
        for (const event of changeEvents) {
          if (event.device_id === id && event.state === "unreviewed") event.state = "ignored";
        }
      }
    }
    return { updated, missing };
  },

  listNetworkScopes(): NetworkScope[] {
    // Most recently used first, matching the backend's ordering.
    return DEMO_SCOPES.map((scope) => ({
      ...scope,
      device_count: [...devices.values()].filter((d) => d.network_scope_id === scope.id).length,
      scan_count: scans.filter((s) => s.network_scope_id === scope.id).length,
    }));
  },

  renameNetworkScope(id: number, name: string): void {
    const scope = DEMO_SCOPES.find((s) => s.id === id);
    if (!scope) throw new Error(`Network ${id} no longer exists.`);
    const trimmed = name.trim();
    if (!trimmed) throw new Error("A network name cannot be empty.");
    scope.display_name = trimmed;
    for (const scan of scans) {
      if (scan.network_scope_id === id) scan.scope_name = trimmed;
    }
  },

  deviceDetail(id: number): DeviceDetail {
    const device = devices.get(id);
    if (!device) throw new Error(`Device ${id} is no longer in the inventory.`);

    const rows: DeviceObservation[] = observationsFor(id).map(({ scan, host }) => ({
      scan_id: scan.id,
      scan_target: scan.target,
      observed_at: host.last_seen,
      ip: host.ip,
      hostname: host.hostname,
      vendor: host.vendor,
      open_ports: host.open_ports,
      response_ms: host.response_ms,
      icmp_ms: host.icmp_ms,
      tcp_ms: host.tcp_ms,
      ttl: host.ttl,
      os_guess: host.os_guess,
    }));

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

    const scope = DEMO_SCOPES.find((s) => s.id === device.network_scope_id);
    return {
      device,
      observations: rows,
      previous_ips: previousIps,
      recent_changes: rows.length >= 2 ? diffFields(asHost(rows[1]), asHost(rows[0])) : [],
      events: [...changeEvents]
        .reverse()
        .filter((e) => e.device_id === id)
        .map(refreshedEvent),
      network_name: scope?.display_name ?? null,
      presence: presenceOf(id, device.network_scope_id),
    };
  },

  setDeviceName(id: number, name: string | null): void {
    const device = requireDevice(id);
    devices.set(id, { ...device, custom_name: name?.trim() || null });
  },

  setDeviceStatus(id: number, status: DeviceStatus): void {
    mock.setDeviceStatuses([id], status);
  },

  setDeviceNotes(id: number, notes: string | null): void {
    const device = requireDevice(id);
    devices.set(id, { ...device, notes: notes?.trim() || null });
  },

  deviceNotes(ids: number[]): Array<[number, string]> {
    const out: Array<[number, string]> = [];
    for (const id of ids) {
      const notes = devices.get(id)?.notes;
      if (notes) out.push([id, notes]);
    }
    return out;
  },

  importDeviceLabels(labels: Record<string, string>): number {
    let adopted = 0;
    for (const [rawMac, label] of Object.entries(labels)) {
      const mac = normalizeMac(rawMac);
      if (!mac) continue;
      for (const device of devices.values()) {
        if (device.mac !== mac) continue;
        devices.set(device.id, {
          ...device,
          custom_name: device.custom_name ?? (label.trim() || null),
          status: device.status === "unclassified" ? "known" : device.status,
        });
        adopted += 1;
      }
    }
    return adopted;
  },

  detectNetworks(): LocalNetwork[] {
    return [
      { interface: "Wi-Fi", ip: "192.168.1.27", prefix: 24, cidr: DEMO_CIDR, is_private: true },
    ];
  },

  /**
   * The demo's public-IP lookup. Runs the real provider fallback against
   * scripted providers, so no request leaves the browser and the address shown
   * is always a documentation address.
   */
  async publicIp(signal?: AbortSignal): Promise<string> {
    publicIpAttempts += 1;
    return lookupPublicIp(scriptedProviderFetch(publicIpScenario(), publicIpAttempts), signal);
  },
};

function requireDevice(id: number): Device {
  const device = devices.get(id);
  if (!device) throw new Error(`Device ${id} is no longer in the inventory.`);
  return device;
}

/**
 * Resolve a scan's scope name at read time, mirroring the backend's join
 * against `network_scopes`. Reading it live rather than trusting the copy
 * stored with the scan is what makes a rename show up across existing history.
 */
function withScopeName<T extends ScanSummary>(summary: T): T {
  const scope = DEMO_SCOPES.find((s) => s.id === summary.network_scope_id);
  return scope ? { ...summary, scope_name: scope.display_name } : summary;
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
