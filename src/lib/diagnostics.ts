// The redacted discovery report, for the browser demo backend.
//
// The packaged app builds this in Rust (`discovery::diagnostics`), straight
// from the database, where the privacy guarantee is structural: the query
// selects no note, no MAC and no serial, so there is nothing to leak. This is
// the mirror the browser demo uses, and it is held to the same rule by the same
// tests: what goes in is a narrow input type with no field for any of it, and
// what comes out drops identifier-bearing evidence a second time regardless.
//
// Nothing here contacts anything, writes a file or records anything. The caller
// puts the string on the clipboard.

import type { Confidence, DeviceDiscovery, DiscoveryQuality, Freshness } from "../types";
import { deviceTypeLabel } from "./discovery";
import { missPhrase, type EffectiveType } from "./effectiveType";

/** Longest report, in characters. Mirrors `MAX_REPORT_CHARS` in Rust. */
export const MAX_REPORT_CHARS = 4_000;

const MAX_EVIDENCE_LINES = 16;
const MAX_VALUE_CHARS = 80;

/**
 * Evidence kinds the report never includes, whatever it is handed.
 *
 * A serial, a description URL and a UPnP UDN identify one unit rather than one
 * kind of device, and an address says where rather than what. None can help fix
 * a classification rule, so none earns the risk.
 */
const EXCLUDED_KINDS = new Set([
  "serial_number",
  "url",
  "protocol_identifier",
  "ipv4_address",
  "ipv6_address",
]);

const CONFIDENCE_WORD: Record<string, string> = {
  high: "High",
  medium: "Medium",
  low: "Low",
  unknown: "Not established",
};

const QUALITY_WORD: Record<string, string> = {
  complete: "Complete",
  limited: "Limited",
  skipped: "Skipped",
  interrupted: "Interrupted",
};

/**
 * Mask an address to its first two octets: `192.168.1.42` becomes
 * `192.168.x.x`. Anything that is not four dot-separated octets is dropped
 * rather than passed through, so an unexpected value cannot escape by failing
 * to match.
 */
export function redactIp(ip: string | null | undefined): string | null {
  const parts = (ip ?? "").trim().split(".");
  if (parts.length !== 4) return null;
  const valid = parts.every((part) => /^\d{1,3}$/.test(part) && Number(part) <= 255);
  if (!valid) return null;
  return `${parts[0]}.${parts[1]}.x.x`;
}

/**
 * Flatten a device-supplied string: control characters become spaces,
 * whitespace collapses, and the result is capped so one hostile value cannot
 * dominate the report.
 */
function clip(value: string): string {
  const collapsed = value
    .replace(/[\u0000-\u001f\u007f]/g, " ")
    .split(/\s+/)
    .filter(Boolean)
    .join(" ");
  if ([...collapsed].length <= MAX_VALUE_CHARS) return collapsed;
  return `${[...collapsed].slice(0, MAX_VALUE_CHARS).join("")}…`;
}

/** Everything the report is allowed to know. No note, MAC, serial or id. */
export interface DiagnosticInput {
  appVersion: string;
  resolved: EffectiveType;
  detectedName?: string | null;
  manufacturer?: string | null;
  model?: string | null;
  /** The OUI manufacturer: a fact about the maker, not about the unit. */
  ouiVendor?: string | null;
  sources?: string[];
  services?: string[];
  evidence?: Array<{
    source: string;
    kind: string;
    value: string;
    freshness: Freshness | string;
    misses: number;
  }>;
  discoveryQuality?: DiscoveryQuality | string | null;
  /** Masked before it reaches the output. */
  ip?: string | null;
}

/**
 * Build the report.
 *
 * Deterministic: the evidence is sorted before anything is written, so two runs
 * over the same device produce identical text and a diff between two reports
 * means something changed.
 */
export function buildDiscoveryReport(input: DiagnosticInput): string {
  const lines: string[] = ["ArcScan discovery report", `Version: ${clip(input.appVersion)}`];
  const { resolved } = input;

  lines.push(`Device type: ${deviceTypeLabel(resolved.effectiveType)}`);
  lines.push(`Type source: ${resolved.isUserSet ? "Set by you" : "Automatic"}`);
  if (resolved.isUserSet) {
    lines.push(`ArcScan detected: ${deviceTypeLabel(resolved.detectedType)}`);
  }
  lines.push(
    `Detected confidence: ${
      CONFIDENCE_WORD[resolved.detectedConfidence] ?? CONFIDENCE_WORD.unknown
    }`,
  );

  const field = (label: string, value: string | null | undefined) => {
    const text = clip(value ?? "");
    if (text) lines.push(`${label}: ${text}`);
  };
  field("Detected name", input.detectedName);
  field("Manufacturer", input.manufacturer);
  field("Model", input.model);
  field("MAC manufacturer", input.ouiVendor);

  const masked = redactIp(input.ip);
  if (masked) lines.push(`Address: ${masked}`);

  const sources = [...new Set((input.sources ?? []).map(clip))].sort();
  if (sources.length > 0) lines.push(`Sources: ${sources.join(", ")}`);

  if (input.discoveryQuality) {
    lines.push(
      `Discovery scan state: ${QUALITY_WORD[input.discoveryQuality] ?? QUALITY_WORD.skipped}`,
    );
  }

  const services = [...new Set((input.services ?? []).map(clip))]
    .sort()
    .slice(0, MAX_EVIDENCE_LINES);
  if (services.length > 0) {
    lines.push("Services:");
    for (const service of services) lines.push(`- ${service}`);
  }

  const usable = (input.evidence ?? [])
    .filter((row) => !EXCLUDED_KINDS.has(row.kind))
    .sort(
      (a, b) =>
        a.source.localeCompare(b.source) ||
        a.kind.localeCompare(b.kind) ||
        a.value.localeCompare(b.value),
    );

  for (const [heading, wantStale] of [
    ["Fresh evidence", false],
    ["Stale evidence", true],
  ] as const) {
    const group = usable
      .filter((row) => (row.freshness === "stale") === wantStale)
      .slice(0, MAX_EVIDENCE_LINES)
      .map((row) => {
        const phrase = row.freshness === "current" ? null : missPhrase(row.misses);
        const suffix = phrase ? ` (${phrase})` : "";
        return `- ${clip(row.source)} ${clip(row.kind)}: ${clip(row.value)}${suffix}`;
      });
    if (group.length === 0) continue;
    lines.push(`${heading}:`);
    lines.push(...group);
  }

  lines.push("");
  lines.push(
    "This report was built on your computer and sent nowhere. " +
      "It deliberately omits your notes, the MAC address, the serial number, " +
      "any device identifier and the full IP address.",
  );

  const text = `${lines.join("\n")}\n`;
  if ([...text].length <= MAX_REPORT_CHARS) return text;
  return `${[...text].slice(0, MAX_REPORT_CHARS).join("")}\n[report truncated]\n`;
}

/** The report for one device, from what the drawer already has loaded. */
export function reportFromDetail(
  appVersion: string,
  resolved: EffectiveType,
  discovery: DeviceDiscovery | null | undefined,
  context: { ouiVendor?: string | null; ip?: string | null; quality?: string | null },
): string {
  return buildDiscoveryReport({
    appVersion,
    resolved,
    detectedName: discovery?.detected_name,
    manufacturer: discovery?.manufacturer,
    model: discovery?.model_name,
    ouiVendor: context.ouiVendor,
    sources: discovery?.sources ?? [],
    services: discovery?.services ?? [],
    evidence: (discovery?.evidence ?? []).map((row) => ({
      source: row.source,
      kind: row.kind,
      value: row.value,
      freshness: row.freshness,
      misses: row.misses,
    })),
    discoveryQuality: context.quality ?? null,
    ip: context.ip,
  });
}

export type { Confidence };
