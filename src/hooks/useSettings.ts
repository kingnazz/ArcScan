import { useCallback, useEffect, useState } from "react";
import { DEFAULT_SETTINGS, loadSettings, saveSettings, type Settings } from "../lib/prefs";

/**
 * The app's settings, persisted on every change.
 *
 * `update` takes a partial so a control can change one field without knowing the
 * rest, which is what stops a stale closure in one panel from reverting a change
 * made in another.
 */
export function useSettings() {
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [loaded, setLoaded] = useState(false);

  // Read on mount rather than in the initialiser: localStorage is not available
  // during server-side or test rendering, and a throw here would blank the app.
  useEffect(() => {
    setSettings(loadSettings());
    setLoaded(true);
  }, []);

  const update = useCallback((patch: Partial<Settings>) => {
    setSettings((current) => {
      const next = { ...current, ...patch };
      saveSettings(next);
      return next;
    });
  }, []);

  const reset = useCallback(() => {
    saveSettings(DEFAULT_SETTINGS);
    setSettings(DEFAULT_SETTINGS);
  }, []);

  // The reduced-motion preference is a class on the root so the CSS rule can
  // switch off every transition at once, matching prefers-reduced-motion.
  useEffect(() => {
    document.documentElement.classList.toggle("reduce-motion", settings.reducedMotion);
  }, [settings.reducedMotion]);

  return { settings, update, reset, loaded };
}
