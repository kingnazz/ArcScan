// Which device type to show, and who decided.
//
// One module, used by the table, the filter, the drawer, the export and the
// search. Precedence reimplemented per component is precedence that disagrees
// with itself, and the disagreement shows up as a row whose type column and
// type filter say different things.
//
// Mirrors `discovery::effective` in Rust, which the diagnostic report uses. The
// two are kept in step by the same arrangement `display_name` has had since
// v1.7: one rule, written twice, tested on both sides.

import type { Confidence, Freshness, InventoryRow, TypeSource } from "../types";
import { CONFIDENCE_LABEL, deviceTypeLabel } from "./discovery";

/**
 * How many consecutive qualifying discovery scans must miss a claim before
 * ArcScan stops leaning on it. Mirrors `STALE_AFTER_MISSES` in Rust, and is
 * here only so the interface can say "three" in words rather than a number
 * nobody chose.
 */
export const STALE_AFTER_MISSES = 3;

/** Everything the rule is allowed to consider. */
export interface TypeInputs {
  /** The operator's correction. `null` or `undefined` means Auto. */
  userOverride?: string | null;
  /** What ArcScan detected, or `null` when discovery has never reached it. */
  detectedType?: string | null;
  /** The detected confidence, as the backend reports it. */
  detectedConfidence?: string | null;
}

/** The answer, and everything a component needs to explain it. */
export interface EffectiveType {
  /** The type to show, filter on and export. */
  effectiveType: string;
  typeSource: TypeSource;
  /** What ArcScan detected, kept underneath so clearing the override is safe. */
  detectedType: string;
  detectedConfidence: string;
  /** True when the operator has made a choice, Unknown included. */
  isUserSet: boolean;
}

/**
 * Settle the type for one device.
 *
 * The whole rule: an override wins, and absent means Auto. An explicit
 * `"unknown"` override is a different thing from Auto — it is a person saying
 * "ArcScan is wrong and I do not know either", which is a real answer that the
 * next scan must not talk them out of.
 *
 * Confidence belongs to the detection and never to the override: ArcScan has no
 * business grading how sure the operator is.
 */
export function resolveType(inputs: TypeInputs): EffectiveType {
  const detectedType = inputs.detectedType ?? "unknown";
  const detectedConfidence = inputs.detectedConfidence ?? "unknown";
  // Only a non-empty string is a choice. An empty string is what an unset
  // `<select>` and a blank database column both look like, and neither is a
  // person deciding anything.
  const chosen = typeof inputs.userOverride === "string" ? inputs.userOverride.trim() : "";
  if (chosen) {
    return {
      effectiveType: chosen,
      typeSource: "user",
      detectedType,
      detectedConfidence,
      isUserSet: true,
    };
  }
  return {
    effectiveType: detectedType,
    typeSource: "automatic",
    detectedType,
    detectedConfidence,
    isUserSet: false,
  };
}

/** The rule applied to an Inventory row, which is where most callers start. */
export function rowType(
  row: Pick<InventoryRow, "discovery" | "user_device_type">,
): EffectiveType {
  return resolveType({
    userOverride: row.user_device_type,
    detectedType: row.discovery?.device_type,
    detectedConfidence: row.discovery?.type_confidence,
  });
}

/** `Set by you` or `Detected automatically`, for a caption under the type. */
export function typeSourceLabel(source: TypeSource): string {
  return source === "user" ? "Set by you" : "Detected automatically";
}

/**
 * The type as one line.
 *
 * A user-set type carries no confidence, because attaching one would be ArcScan
 * grading the operator. An automatic Unknown carries none either, since
 * "Unknown · Not established" says the same thing twice.
 */
export function effectiveTypeSummary(resolved: EffectiveType): string {
  const label = deviceTypeLabel(resolved.effectiveType);
  if (resolved.isUserSet) return label;
  if (resolved.effectiveType === "unknown") return label;
  return `${label} · ${CONFIDENCE_LABEL[resolved.detectedConfidence] ?? CONFIDENCE_LABEL.unknown}`;
}

/**
 * What ArcScan detected, for the line underneath an override.
 *
 * `null` when there is nothing worth saying: no override, or an override on a
 * device discovery never reached, where "ArcScan detected: Unknown" is noise.
 */
export function detectedUnderOverride(resolved: EffectiveType): string | null {
  if (!resolved.isUserSet) return null;
  if (resolved.detectedType === "unknown") return null;
  const label = deviceTypeLabel(resolved.detectedType);
  const confidence = CONFIDENCE_LABEL[resolved.detectedConfidence];
  return confidence ? `${label} · ${confidence}` : label;
}

/** The words for each freshness state, as a badge. */
export const FRESHNESS_LABEL: Record<string, string> = {
  current: "Current",
  aging: "Getting old",
  stale: "Stale",
};

/**
 * What each freshness state means, spelled out.
 *
 * Counted in scans on purpose, and the hint says so: a person whose last scan
 * was in March should not read "stale" as "three months old".
 */
export const FRESHNESS_HINT: Record<string, string> = {
  current: "Confirmed by the most recent scan that could have heard it.",
  aging: "Not heard by the last scan or two that could have heard it. Still believed.",
  stale: `Not heard by ${STALE_AFTER_MISSES} scans in a row that could have heard it. Kept and shown, but no longer enough on its own to make ArcScan sure.`,
};

export function freshnessLabel(value: string | null | undefined): string {
  if (!value) return FRESHNESS_LABEL.current;
  return FRESHNESS_LABEL[value] ?? FRESHNESS_LABEL.current;
}

/** True when a state is worth drawing attention to. `current` is not. */
export function isNoteworthyFreshness(value: string | null | undefined): boolean {
  return value === "aging" || value === "stale";
}

/**
 * "last seen 4 discovery scans ago", or `null` for a claim heard this time.
 *
 * Scans, never days. A count of scans is a fact ArcScan observed; a number of
 * days is a fact about the operator's calendar.
 */
export function missPhrase(misses: number | null | undefined): string | null {
  const count = typeof misses === "number" ? Math.floor(misses) : 0;
  if (count < 1) return null;
  return count === 1
    ? "last seen 1 discovery scan ago"
    : `last seen ${count} discovery scans ago`;
}

export type { Confidence, Freshness, TypeSource };
