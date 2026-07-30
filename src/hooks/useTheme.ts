import { useEffect, useState } from "react";
import type { ThemePref } from "../lib/prefs";

export type ResolvedTheme = "light" | "dark";

/**
 * Apply the theme preference to the document.
 *
 * "system" follows the OS and keeps following it, which is what a desktop utility
 * should do by default. The resolved value is returned so the toggle can show the
 * theme actually in effect rather than the word "system".
 */
export function useTheme(preference: ThemePref): ResolvedTheme {
  const [systemDark, setSystemDark] = useState(
    () =>
      typeof window !== "undefined" &&
      Boolean(window.matchMedia?.("(prefers-color-scheme: dark)").matches),
  );

  useEffect(() => {
    const query = window.matchMedia?.("(prefers-color-scheme: dark)");
    if (!query) return;
    const onChange = (event: MediaQueryListEvent) => setSystemDark(event.matches);
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);

  const resolved: ResolvedTheme =
    preference === "system" ? (systemDark ? "dark" : "light") : preference;

  useEffect(() => {
    document.documentElement.classList.toggle("dark", resolved === "dark");
    // Mirrored into localStorage so the inline script in index.html can apply the
    // theme before first paint and avoid a flash of the wrong one.
    try {
      localStorage.setItem("arcscan-theme", preference);
    } catch {
      // Storage unavailable. The theme still applies for this session.
    }
  }, [resolved, preference]);

  return resolved;
}
