// Lightweight client-side preferences persisted in localStorage. Works in both
// the native Tauri build (WebView localStorage persists) and the browser demo,
// with no backend involvement.

const KNOWN_KEY = "arcscan-known-devices";
const RANGES_KEY = "arcscan-recent-ranges";
const MAX_RANGES = 8;

// MAC (uppercase, colon-separated) -> user label. Presence in the map means the
// device is "known"/favorited; the label may be empty.
export type KnownMap = Record<string, string>;

function normMac(mac: string): string {
  return mac.trim().toUpperCase();
}

export function loadKnown(): KnownMap {
  try {
    const raw = localStorage.getItem(KNOWN_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? (parsed as KnownMap) : {};
  } catch {
    return {};
  }
}

function saveKnown(map: KnownMap) {
  try {
    localStorage.setItem(KNOWN_KEY, JSON.stringify(map));
  } catch {
    /* ignore quota/availability errors */
  }
}

export function isKnown(map: KnownMap, mac: string | null): boolean {
  return !!mac && normMac(mac) in map;
}

export function labelFor(map: KnownMap, mac: string | null): string {
  return mac ? (map[normMac(mac)] ?? "") : "";
}

/** Add/remove a device from the known list. Returns a new map. */
export function toggleKnown(map: KnownMap, mac: string, defaultLabel = ""): KnownMap {
  const key = normMac(mac);
  const next = { ...map };
  if (key in next) {
    delete next[key];
  } else {
    next[key] = defaultLabel;
  }
  saveKnown(next);
  return next;
}

/** Set (or update) a device's label, marking it known. Returns a new map. */
export function setLabel(map: KnownMap, mac: string, label: string): KnownMap {
  const next = { ...map, [normMac(mac)]: label };
  saveKnown(next);
  return next;
}

export function loadRanges(): string[] {
  try {
    const raw = localStorage.getItem(RANGES_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed.filter((r) => typeof r === "string") : [];
  } catch {
    return [];
  }
}

/** Record a scanned target as most-recent (deduped, capped). Returns the list. */
export function pushRange(target: string): string[] {
  const t = target.trim();
  if (!t) return loadRanges();
  const list = [t, ...loadRanges().filter((r) => r !== t)].slice(0, MAX_RANGES);
  try {
    localStorage.setItem(RANGES_KEY, JSON.stringify(list));
  } catch {
    /* ignore */
  }
  return list;
}
