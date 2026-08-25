// How discovery reads on screen.
//
// The backend decides the facts — what a device advertised, what type it is,
// how sure that is. Everything here is the wording those facts get, kept in one
// place so the table, the drawer, the inbox and the export cannot describe the
// same thing three different ways.
//
// Every value that reaches this module came off the network from an
// unauthenticated device. It arrives already bounded and stripped of control
// characters by the Rust parsers, and React renders it as text, so nothing here
// needs to escape anything — but nothing here may ever put a device-supplied
// string somewhere it would be interpreted, either.

import type {
  Confidence,
  DeviceDiscovery,
  DiscoveryMode,
  DiscoveryQuality,
  DiscoveryReport,
  InventoryDiscovery,
  InventoryRow,
} from "../types";

/** The words for each device type. Mirrors `DeviceType::label` in Rust. */
export const DEVICE_TYPE_LABEL: Record<string, string> = {
  router: "Router",
  printer: "Printer",
  computer: "Computer",
  phone: "Phone",
  tablet: "Tablet",
  television: "Television",
  media_device: "Media device",
  camera: "Camera",
  nas: "NAS",
  game_console: "Game console",
  smart_home: "Smart-home device",
  network_equipment: "Network equipment",
  speaker: "Speaker",
  unknown: "Unknown",
};

/** A type ArcScan does not recognise reads as its raw value, never as blank. */
export function deviceTypeLabel(id: string | null | undefined): string {
  if (!id) return DEVICE_TYPE_LABEL.unknown;
  return DEVICE_TYPE_LABEL[id] ?? id;
}

export const CONFIDENCE_LABEL: Record<string, string> = {
  high: "High confidence",
  medium: "Medium confidence",
  low: "Low confidence",
  unknown: "Not established",
};

/**
 * What each confidence word actually means, shown as a tooltip.
 *
 * Written out because "medium" on its own tells a person nothing about whether
 * to act on it, which is the only question they are asking.
 */
export const CONFIDENCE_HINT: Record<string, string> = {
  high: "The device declared this through a protocol built for the purpose, and something independent agrees.",
  medium: "One protocol-level declaration, with nothing corroborating it.",
  low: "Inferred from an open port, a manufacturer or a name. Worth knowing, not worth acting on.",
  unknown: "Nothing ArcScan saw supports a device type.",
};

export function confidenceLabel(value: string | null | undefined): string {
  if (!value) return CONFIDENCE_LABEL.unknown;
  return CONFIDENCE_LABEL[value] ?? CONFIDENCE_LABEL.unknown;
}

/** Where a fact came from, in words a person recognises. */
export const SOURCE_LABEL: Record<string, string> = {
  user: "You named it",
  mdns: "mDNS",
  ssdp: "SSDP",
  reverse_dns: "Reverse DNS",
  arp_vendor: "MAC manufacturer",
  tcp_service: "Open port",
  scan_observation: "Scan",
};

export function sourceLabel(value: string | null | undefined): string {
  if (!value) return "Unknown";
  return SOURCE_LABEL[value] ?? value;
}

/** The sources a device was seen through, as one readable line. */
export function sourcesLabel(sources: string[]): string {
  const named = sources.filter((s) => s !== "user").map(sourceLabel);
  return named.length > 0 ? named.join(" · ") : "—";
}

/** What kind of claim an evidence row is. */
export const EVIDENCE_KIND_LABEL: Record<string, string> = {
  display_name: "Name",
  hostname: "Host name",
  manufacturer: "Manufacturer",
  model: "Model",
  model_number: "Model number",
  serial_number: "Serial number",
  device_type: "Device type",
  service: "Service",
  service_port: "Service port",
  url: "Address",
  ipv4_address: "IPv4 address",
  ipv6_address: "IPv6 address",
  protocol_identifier: "Protocol identifier",
};

export function evidenceKindLabel(kind: string): string {
  return EVIDENCE_KIND_LABEL[kind] ?? kind;
}

/**
 * A device type and how sure it is, as one line: `Printer · High confidence`.
 *
 * An unestablished type reads as a plain "Unknown" with no confidence attached,
 * because "Unknown · Not established" says the same thing twice.
 */
export function typeSummary(
  deviceType: string | null | undefined,
  confidence: string | null | undefined,
): string {
  const type = deviceTypeLabel(deviceType);
  if (!deviceType || deviceType === "unknown") return DEVICE_TYPE_LABEL.unknown;
  return `${type} · ${confidenceLabel(confidence)}`;
}

/**
 * Tidy an advertised service type for display: `_ipp._tcp` reads as `IPP`, and
 * anything unrecognised keeps its own name rather than being hidden.
 */
const SERVICE_LABEL: Record<string, string> = {
  _ipp: "IPP printing",
  _ipps: "IPP printing (secure)",
  _printer: "Line printer",
  "_pdl-datastream": "Raw printing",
  _scanner: "Scanner",
  _http: "Web",
  _https: "Web (secure)",
  _smb: "File sharing",
  _ssh: "SSH",
  _sftp: "SFTP",
  _afpovertcp: "Apple file sharing",
  _workstation: "Workstation",
  "_device-info": "Device information",
  _airplay: "AirPlay",
  _raop: "AirPlay audio",
  _googlecast: "Chromecast",
  _spotify: "Spotify Connect",
  "_spotify-connect": "Spotify Connect",
  _sonos: "Sonos",
  _hap: "HomeKit",
  _homekit: "HomeKit",
  _matter: "Matter",
  _matterc: "Matter commissioning",
  _rfb: "Screen sharing",
  _rtsp: "Video stream",
  _daap: "Media library",
  _dacp: "Media control",
  MediaRenderer: "Media playback",
  MediaServer: "Media library",
  ContentDirectory: "Media library",
  WANIPConnection: "Internet gateway",
  WANCommonInterfaceConfig: "Internet gateway",
  Layer3Forwarding: "Routing",
  AVTransport: "Media playback",
  RenderingControl: "Media playback",
};

export function serviceName(service: string): string {
  const trimmed = service.trim();
  if (!trimmed) return "";
  const known = SERVICE_LABEL[trimmed];
  if (known) return known;
  // mDNS service types are `_name._proto`; take the leading label.
  const head = trimmed.split(".")[0];
  return SERVICE_LABEL[head] ?? trimmed;
}

/** Services as one compact line, capped so a chatty device cannot dominate. */
export function servicesLabel(services: string[], limit = 4): string {
  if (services.length === 0) return "—";
  const shown = services.slice(0, limit).map(serviceName);
  const rest = services.length - shown.length;
  return rest > 0 ? `${shown.join(", ")}, +${rest} more` : shown.join(", ");
}

/** What a scan's discovery pass managed, for History and scan detail. */
export const DISCOVERY_MODE_LABEL: Record<string, string> = {
  full: "mDNS + SSDP",
  partial: "Local discovery incomplete",
  none: "No local discovery",
};

export function discoveryModeLabel(mode: DiscoveryMode | string): string {
  return DISCOVERY_MODE_LABEL[mode] ?? DISCOVERY_MODE_LABEL.none;
}

/**
 * How well a scan's discovery pass went, in one word.
 *
 * Four states rather than three, because "it ran and heard nothing" and "it ran
 * and could not finish" are different facts and a person reading History needs
 * to know which they are looking at.
 */
export const DISCOVERY_QUALITY_LABEL: Record<string, string> = {
  complete: "Complete",
  limited: "Limited",
  skipped: "Skipped",
  interrupted: "Interrupted",
};

export const DISCOVERY_QUALITY_HINT: Record<string, string> = {
  complete: "Both protocols ran and finished, and nothing was cut short.",
  limited:
    "Discovery ran but could not do all of it. What ArcScan observed is shown beside this.",
  skipped: "Discovery did not run: a remote target, switched off, or no local interface to send from.",
  interrupted: "The scan was stopped while discovery was still in progress.",
};

export function discoveryQualityLabel(quality: DiscoveryQuality | string | null | undefined): string {
  if (!quality) return DISCOVERY_QUALITY_LABEL.skipped;
  return DISCOVERY_QUALITY_LABEL[quality] ?? DISCOVERY_QUALITY_LABEL.skipped;
}

/**
 * The one compact line History shows: what the pass managed, and either the
 * counts it heard or the single thing that limited it.
 *
 *   `Complete · 12 mDNS · 8 SSDP`
 *   `Skipped · Remote scan`
 *
 * Counts only when the pass was complete: a number of responses beside the word
 * "Limited" invites a reader to treat the number as the whole story, and it is
 * exactly the case where it is not.
 */
export function discoverySummaryLine(scan: {
  discovery_quality?: DiscoveryQuality | string | null;
  discovery_quality_reason?: string | null;
  discovery_summary?: string | null;
}): string {
  const quality = scan.discovery_quality ?? "skipped";
  const parts: string[] = [discoveryQualityLabel(quality)];
  if (quality === "complete") {
    const report = parseDiscoveryReport(scan.discovery_summary);
    if (report) {
      parts.push(`${report.mdns_responses} mDNS`);
      parts.push(`${report.ssdp_responses} SSDP`);
    }
  } else if (scan.discovery_quality_reason) {
    parts.push(scan.discovery_quality_reason);
  }
  return parts.join(" · ");
}

/**
 * The stored discovery summary, parsed defensively.
 *
 * The column holds whatever the build that wrote the scan put there, so a row
 * from a newer or a corrupted build must read as "no summary" rather than
 * throwing inside a list render.
 */
export function parseDiscoveryReport(raw: string | null | undefined): DiscoveryReport | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as Partial<DiscoveryReport>;
    if (!parsed || typeof parsed !== "object") return null;
    return {
      mdns_attempted: Boolean(parsed.mdns_attempted),
      ssdp_attempted: Boolean(parsed.ssdp_attempted),
      mdns_responses: Number(parsed.mdns_responses ?? 0),
      ssdp_responses: Number(parsed.ssdp_responses ?? 0),
      descriptions_fetched: Number(parsed.descriptions_fetched ?? 0),
      descriptions_rejected: Number(parsed.descriptions_rejected ?? 0),
      description_notes: Array.isArray(parsed.description_notes) ? parsed.description_notes : [],
      devices_enriched: Number(parsed.devices_enriched ?? 0),
      duration_ms: Number(parsed.duration_ms ?? 0),
      skip_reason: parsed.skip_reason ?? null,
      interrupted: Boolean(parsed.interrupted),
      mdns_socket_failed: Boolean(parsed.mdns_socket_failed),
      ssdp_socket_failed: Boolean(parsed.ssdp_socket_failed),
      mdns_capped: Boolean(parsed.mdns_capped),
      ssdp_capped: Boolean(parsed.ssdp_capped),
      descriptions_capped: Boolean(parsed.descriptions_capped),
    };
  } catch {
    return null;
  }
}

/**
 * Everything about a row that a search term should be able to reach.
 *
 * Deliberately includes both the raw service type (`_ipp._tcp`) and its
 * friendly name (`IPP printing`), so a technician searching for the protocol
 * and someone searching for the word both find the printer.
 */
export function discoveryHaystack(discovery: InventoryDiscovery | null | undefined): string {
  if (!discovery) return "";
  return [
    discovery.detected_name ?? "",
    // The *detected* type, which stays searchable under an override: a
    // technician looking for "everything ArcScan called a media device" is
    // asking a real question, and a correction should not hide the answer.
    discovery.device_type,
    deviceTypeLabel(discovery.device_type),
    discovery.type_confidence,
    discovery.manufacturer ?? "",
    discovery.model_name ?? "",
    discovery.services.join(" "),
    discovery.services.map(serviceName).join(" "),
    discovery.sources.map(sourceLabel).join(" "),
    // So a search for "stale" finds the devices whose evidence has gone quiet.
    discovery.evidence_freshness ?? "",
  ].join(" ");
}

/**
 * The name to show for a device, matching `display_name_detected` in Rust.
 *
 * A name the operator typed wins, always. A name the device advertised comes
 * next, then the reverse-DNS hostname, then the manufacturer with the address,
 * then the address alone.
 */
export function resolveDisplayName(row: {
  custom_name: string | null;
  hostname: string | null;
  vendor: string | null;
  current_ip: string | null;
  discovery?: InventoryDiscovery | null;
}): string {
  const pick = (value: string | null | undefined) => {
    const trimmed = value?.trim();
    return trimmed ? trimmed : null;
  };
  const ip = pick(row.current_ip) ?? "";
  return (
    pick(row.custom_name) ??
    pick(row.discovery?.detected_name) ??
    pick(row.hostname) ??
    (pick(row.vendor) ? `${row.vendor?.trim()} (${ip})` : null) ??
    ip
  );
}

/** True when the name on screen is one the operator typed. */
export function hasUserName(row: Pick<InventoryRow, "custom_name">): boolean {
  return Boolean(row.custom_name?.trim());
}

/**
 * Whether a detected name is being shown *instead of* something the device also
 * advertised, so the drawer can say so rather than silently picking one.
 */
export function hasNameConflict(discovery: DeviceDiscovery | null | undefined): boolean {
  if (!discovery) return false;
  return discovery.alternate_names.length > 0;
}

/** Confidence as a badge tone, so weak claims never look authoritative. */
export function confidenceTone(confidence: string | null | undefined): "online" | "accent" | "neutral" {
  if (confidence === "high") return "online";
  if (confidence === "medium") return "accent";
  return "neutral";
}

export type { Confidence };
