import { useEffect, useState } from "react";

// A public IP looks like an IPv4 dotted quad or an IPv6 (contains a colon).
function looksLikeIp(s: string): boolean {
  return /^(\d{1,3}\.){3}\d{1,3}$/.test(s) || (/:/.test(s) && /^[0-9a-fA-F:.]+$/.test(s));
}

/**
 * Look up this machine's public IP address from a CORS-enabled service, but
 * only if the internet is reachable. Returns null while loading or when the
 * lookup fails (e.g. offline) — the UI simply hides the field in that case.
 * This makes a single outbound request to fetch the address; no scan data or
 * other information is ever sent.
 */
export function usePublicIp(): string | null {
  const [ip, setIp] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 5000);

    const sources: Array<() => Promise<string>> = [
      async () => {
        const r = await fetch("https://api64.ipify.org?format=json", { signal: controller.signal });
        if (!r.ok) throw new Error(String(r.status));
        return String((await r.json()).ip ?? "").trim();
      },
      async () => {
        const r = await fetch("https://icanhazip.com", { signal: controller.signal });
        if (!r.ok) throw new Error(String(r.status));
        return (await r.text()).trim();
      },
    ];

    (async () => {
      for (const source of sources) {
        try {
          const value = await source();
          if (looksLikeIp(value)) {
            if (!cancelled) setIp(value);
            break;
          }
        } catch {
          // Try the next source, or give up silently (most likely offline).
        }
      }
      clearTimeout(timer);
    })();

    return () => {
      cancelled = true;
      controller.abort();
      clearTimeout(timer);
    };
  }, []);

  return ip;
}
