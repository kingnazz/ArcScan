// What the scan status line says.
//
// The line under the scan command narrates one scan from end to end: what will
// happen before it starts, what is happening while it runs, and what happened
// once it stops. Keeping that in one place — and in a pure function — is what
// makes empty → scanning → results read as one continuous surface, and what
// lets every state be tested without rendering anything.

import { formatCount, formatDateTime, formatDuration } from "./format";
import type { LocalNetwork, ScanComparison, ScanPhase, ScanPreview, ScanProgress } from "../types";
import type { ScanMeta, ScanMode } from "../hooks/useLiveScan";

/** The three stages a person can follow, folded from the backend's phases. */
export const SCAN_STAGES = ["Finding hosts", "Identifying devices", "Resolving names"] as const;

/**
 * Which stage a backend phase belongs to, 1-based, or 0 before the first
 * progress event.
 *
 * Six phases are real but too fine to follow: confirming quiet devices is part
 * of finding hosts, and discovery, description and classification are all
 * "working out what each device is".
 */
export function stageOf(phase: ScanPhase | string | null | undefined): 0 | 1 | 2 | 3 {
  switch (phase) {
    case "probing":
    case "confirming":
      return 1;
    case "discovering":
    case "describing":
    case "classifying":
      return 2;
    case "resolving":
    case "done":
      return 3;
    default:
      return 0;
  }
}

export type StatusTone = "neutral" | "live" | "success" | "warning";

export type StatusIcon =
  | "network"
  | "routed"
  | "live"
  | "stopping"
  | "complete"
  | "stopped"
  | "empty"
  | "saved";

export interface StatusLine {
  tone: StatusTone;
  icon: StatusIcon;
  /** The short verdict, shown in the strongest weight. */
  headline: string;
  /** Supporting facts, joined with middle dots. */
  details: string[];
  /** A comparison summary the operator can open, when there is one. */
  comparison?: { text: string; attention: boolean } | null;
  /** One quiet sentence at the end: a hint or a caution. */
  note?: { text: string; tone: "muted" | "warning" } | null;
}

export interface StatusInput {
  mode: ScanMode;
  stopping: boolean;
  progress: ScanProgress | null;
  meta: ScanMeta | null;
  comparison: ScanComparison | null;
  /** Rows on screen for this scan. */
  deviceCount: number;
  /** What is in the target field now. */
  target: string;
  preview: ScanPreview | null;
  localNetworks: LocalNetwork[];
  /** Set when the chosen depth needs a Windows credential and none is set. */
  credentialMissing?: boolean;
}

function plural(count: number, one: string, many = `${one}s`): string {
  return `${formatCount(count)} ${count === 1 ? one : many}`;
}

/** The local network a target names exactly, if any. */
export function localNetworkFor(target: string, networks: LocalNetwork[]): LocalNetwork | null {
  const value = target.trim();
  return networks.find((network) => network.cidr === value) ?? null;
}

/**
 * Summarise a comparison in one phrase.
 *
 * Returns null while the scan is still being saved, because saying "no
 * changes" before the comparison exists would be a guess.
 */
export function comparisonPhrase(
  comparison: ScanComparison | null,
): { text: string; attention: boolean } | null {
  if (!comparison) return null;
  if (comparison.baseline_scan_id == null) {
    return { text: "First scan of this target — nothing to compare yet", attention: false };
  }
  const added = comparison.added.filter((d) => d.kind === "new").length;
  const back = comparison.added.filter((d) => d.kind === "returned").length;
  const parts: string[] = [];
  if (added > 0) parts.push(`${added} new`);
  if (back > 0) parts.push(`${back} returned`);
  if (comparison.changed.length > 0) parts.push(`${comparison.changed.length} changed`);
  if (comparison.removed.length > 0) parts.push(`${comparison.removed.length} missing`);
  const since = comparison.baseline_created_at
    ? ` since ${formatDateTime(comparison.baseline_created_at)}`
    : " since the previous scan";
  if (parts.length === 0) return { text: `No changes${since}`, attention: false };
  return { text: `${parts.join(", ")}${since}`, attention: true };
}

/**
 * True when the line should describe the scan about to run rather than the one
 * on screen: nothing has run yet, or the operator has typed a different target.
 */
export function showsPreflight(input: Pick<StatusInput, "mode" | "meta" | "target">): boolean {
  if (input.mode === "scanning") return false;
  if (!input.meta) return true;
  const typed = input.target.trim();
  return typed.length > 0 && typed !== input.meta.target;
}

export function statusLine(input: StatusInput): StatusLine {
  const { mode, meta, progress } = input;

  if (mode === "scanning") {
    if (input.stopping) {
      return {
        tone: "warning",
        icon: "stopping",
        headline: "Stopping",
        details: [`keeping the ${plural(input.deviceCount, "device")} found so far`],
      };
    }
    const stage = stageOf(progress?.phase);
    const details: string[] = [];
    if (meta?.target) details.push(meta.target);
    details.push(stage === 0 ? "Starting" : `Step ${stage} of 3: ${SCAN_STAGES[stage - 1]}`);
    if (progress && progress.total > 0 && stage === 1) {
      details.push(`${formatCount(progress.done)} of ${formatCount(progress.total)} checked`);
    }
    details.push(`${plural(progress?.found ?? input.deviceCount, "device")} found`);
    return {
      tone: "live",
      icon: "live",
      headline: "Scanning",
      details,
      note: { text: "Stop keeps everything found so far.", tone: "muted" },
    };
  }

  if (showsPreflight(input)) {
    const target = input.target.trim();
    if (!target) {
      return {
        tone: "neutral",
        icon: "network",
        headline: "Ready",
        details: ["Enter an address, a range such as 10.0.0.1-50, or a CIDR block"],
      };
    }
    const local = localNetworkFor(target, input.localNetworks);
    const details: string[] = [];
    if (local) details.push(`${local.interface} · this computer is ${local.ip}`);
    if (input.preview) {
      details.push(plural(input.preview.total, "address", "addresses"));
      details.push(plural(input.preview.port_count, "port"));
    }
    const note = input.credentialMissing
      ? {
          text: "No Windows credential is set, so Windows machines will be scanned as Deep.",
          tone: "warning" as const,
        }
      : input.preview?.warning
        ? { text: input.preview.warning, tone: "warning" as const }
        : local
          ? null
          : {
              text: "MAC addresses and vendors are only visible on this computer's own network.",
              tone: "muted" as const,
            };
    return {
      tone: "neutral",
      icon: local ? "network" : "routed",
      headline: local ? "Local network" : "Routed or custom target",
      details,
      note,
    };
  }

  // A scan on screen: finished just now, or reopened from History.
  const m = meta as ScanMeta;
  if (m.cancelled) {
    return {
      tone: "warning",
      icon: "stopped",
      headline: "Stopped early",
      details: [
        `${plural(input.deviceCount, "device")} in ${formatCount(m.probed)} of ${formatCount(m.scanned)} addresses`,
      ],
      note: {
        text: "Saved as a partial scan. Partial scans are never compared, so nothing is reported missing.",
        tone: "muted",
      },
    };
  }

  const timing = `${plural(m.scanned, "address", "addresses")} in ${formatDuration(m.durationMs)}`;
  if (input.deviceCount === 0) {
    return {
      tone: "neutral",
      icon: "empty",
      headline: "No devices answered",
      details: [m.target, timing],
    };
  }

  return {
    tone: mode === "history" ? "neutral" : "success",
    icon: mode === "history" ? "saved" : "complete",
    headline: mode === "history" ? "Saved scan" : "Scan complete",
    details: [plural(input.deviceCount, "device"), m.target, timing],
    comparison: comparisonPhrase(input.comparison),
  };
}
