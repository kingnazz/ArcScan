// Local preferences.
//
// Persisted in localStorage, which survives in both the native WebView and the
// browser demo with no backend involvement. Device names, notes and status moved
// into the database in v1.7 (they are inventory data, not preferences); what
// stays here is genuinely per-installation UI state.
//
// Every read is defensive: a preferences blob written by a newer build, or
// corrupted on disk, must never stop the app from starting.

import { isProfileId, type ProfileId } from "./profiles";
import type { InventoryColumn } from "./inventory";
import type { SortDir, SortKey } from "./table";

const SETTINGS_KEY = "arcscan-settings";
const RECENTS_KEY = "arcscan-recent-targets";
/** v1.6 kept device labels here, keyed by MAC. Read once, then migrated. */
export const LEGACY_KNOWN_KEY = "arcscan-known-devices";
const LABEL_MIGRATION_KEY = "arcscan-labels-imported";

const MAX_RECENTS = 8;

export type ThemePref = "light" | "dark" | "system";
export type Density = "compact" | "comfortable";

export interface Settings {
  theme: ThemePref;
  defaultProfile: ProfileId;
  /** Port specification used by the Custom and Full TCP profiles. */
  portSpec: string;
  timeoutMs: number;
  hostConcurrency: number;
  tcpConcurrency: number;
  pingConcurrency: number;
  density: Density;
  hiddenColumns: SortKey[];
  /** Optional Inventory columns the operator has turned on. */
  inventoryColumns: InventoryColumn[];
  sortKey: SortKey;
  sortDir: SortDir;
  /** Scans kept before the oldest are pruned. */
  historyRetention: number;
  /** Off by default: the lookup contacts a third party, so it is opt-in. */
  publicIpLookup: boolean;
  checkForUpdates: boolean;
  notifyOnChanges: boolean;
  reducedMotion: boolean;
  /** Cleared once the operator has seen the first-run guidance. */
  showFirstRunGuidance: boolean;
}

export const DEFAULT_SETTINGS: Settings = {
  theme: "system",
  defaultProfile: "quick-lan",
  portSpec: "21, 22, 23, 53, 80, 110, 139, 143, 443, 445, 3389, 5900, 8080, 8443",
  timeoutMs: 900,
  hostConcurrency: 64,
  tcpConcurrency: 256,
  pingConcurrency: 32,
  density: "compact",
  hiddenColumns: [],
  inventoryColumns: [],
  sortKey: "ip",
  sortDir: "asc",
  historyRetention: 100,
  publicIpLookup: false,
  checkForUpdates: true,
  notifyOnChanges: true,
  reducedMotion: false,
  showFirstRunGuidance: true,
};

/** Optional Inventory columns, for validating what was stored. */
const INVENTORY_COLUMN_KEYS: InventoryColumn[] = [
  "mac",
  "hostname",
  "first_seen",
  "observations",
  "response",
  "previous",
];

const SORT_KEYS: SortKey[] = [
  "state",
  "name",
  "ip",
  "mac",
  "vendor",
  "os",
  "ports",
  "response",
  "last_seen",
];

function readJson(key: string): unknown {
  try {
    const raw = localStorage.getItem(key);
    return raw ? JSON.parse(raw) : null;
  } catch {
    return null;
  }
}

function writeJson(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Quota exhausted or storage unavailable. Preferences are a convenience, so
    // the app carries on with whatever is in memory for this session.
  }
}

/** Coerce a stored value into a number inside a range, or fall back. */
function clampNumber(value: unknown, min: number, max: number, fallback: number): number {
  const n = typeof value === "number" ? value : Number.NaN;
  if (!Number.isFinite(n)) return fallback;
  return Math.min(max, Math.max(min, Math.round(n)));
}

function oneOf<T extends string>(value: unknown, allowed: readonly T[], fallback: T): T {
  return typeof value === "string" && (allowed as readonly string[]).includes(value)
    ? (value as T)
    : fallback;
}

/** Read the settings, filling in anything missing or invalid from the defaults. */
export function loadSettings(): Settings {
  const raw = readJson(SETTINGS_KEY);
  if (!raw || typeof raw !== "object") return { ...DEFAULT_SETTINGS };
  const stored = raw as Record<string, unknown>;
  const d = DEFAULT_SETTINGS;

  return {
    theme: oneOf(stored.theme, ["light", "dark", "system"] as const, d.theme),
    defaultProfile: isProfileId(stored.defaultProfile)
      ? (stored.defaultProfile as ProfileId)
      : d.defaultProfile,
    portSpec: typeof stored.portSpec === "string" && stored.portSpec.trim() ? stored.portSpec : d.portSpec,
    timeoutMs: clampNumber(stored.timeoutMs, 50, 10_000, d.timeoutMs),
    hostConcurrency: clampNumber(stored.hostConcurrency, 1, 1_024, d.hostConcurrency),
    tcpConcurrency: clampNumber(stored.tcpConcurrency, 8, 2_048, d.tcpConcurrency),
    pingConcurrency: clampNumber(stored.pingConcurrency, 1, 128, d.pingConcurrency),
    density: oneOf(stored.density, ["compact", "comfortable"] as const, d.density),
    hiddenColumns: Array.isArray(stored.hiddenColumns)
      ? (stored.hiddenColumns.filter((c): c is SortKey =>
          SORT_KEYS.includes(c as SortKey),
        ) as SortKey[])
      : d.hiddenColumns,
    inventoryColumns: Array.isArray(stored.inventoryColumns)
      ? (stored.inventoryColumns.filter((c): c is InventoryColumn =>
          INVENTORY_COLUMN_KEYS.includes(c as InventoryColumn),
        ) as InventoryColumn[])
      : d.inventoryColumns,
    sortKey: oneOf(stored.sortKey, SORT_KEYS, d.sortKey),
    sortDir: oneOf(stored.sortDir, ["asc", "desc"] as const, d.sortDir),
    historyRetention: clampNumber(stored.historyRetention, 5, 5_000, d.historyRetention),
    publicIpLookup: stored.publicIpLookup === true,
    checkForUpdates: stored.checkForUpdates !== false,
    notifyOnChanges: stored.notifyOnChanges !== false,
    reducedMotion: stored.reducedMotion === true,
    showFirstRunGuidance: stored.showFirstRunGuidance !== false,
  };
}

export function saveSettings(settings: Settings): void {
  writeJson(SETTINGS_KEY, settings);
}

// --- Recent targets --------------------------------------------------------

export function loadRecentTargets(): string[] {
  const raw = readJson(RECENTS_KEY);
  return Array.isArray(raw) ? raw.filter((t): t is string => typeof t === "string") : [];
}

/** Record a target as most recent, de-duplicated and capped. */
export function pushRecentTarget(target: string): string[] {
  const t = target.trim();
  if (!t) return loadRecentTargets();
  const list = [t, ...loadRecentTargets().filter((r) => r !== t)].slice(0, MAX_RECENTS);
  writeJson(RECENTS_KEY, list);
  return list;
}

export function clearRecentTargets(): string[] {
  writeJson(RECENTS_KEY, []);
  return [];
}

// --- v1.6 device-label migration ------------------------------------------

/**
 * The device labels v1.6 stored in localStorage, keyed by MAC.
 *
 * Returns null once the import has already run, so an operator who deliberately
 * renamed a device in v1.7 does not get the old label pushed back over it on
 * every launch.
 */
export function pendingLegacyLabels(): Record<string, string> | null {
  try {
    if (localStorage.getItem(LABEL_MIGRATION_KEY) === "done") return null;
  } catch {
    return null;
  }
  const raw = readJson(LEGACY_KNOWN_KEY);
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  const entries = Object.entries(raw as Record<string, unknown>).filter(
    (entry): entry is [string, string] => typeof entry[1] === "string",
  );
  return entries.length > 0 ? Object.fromEntries(entries) : null;
}

export function markLegacyLabelsImported(): void {
  try {
    localStorage.setItem(LABEL_MIGRATION_KEY, "done");
  } catch {
    // Without the marker the import runs again next launch, which is harmless:
    // it only fills gaps and never overwrites a name set since.
  }
}
