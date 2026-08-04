// Public-IP lookup: the providers, the fallback order, and the one function
// that talks to them.
//
// This is the only place in ArcScan that contacts a host outside the operator's
// own network on purpose, so it is kept small, explicit and separately
// testable. Nothing here runs unless a person presses Check: there is no
// module-level call, no timer and no cache warm-up.
//
// What leaves the machine is a bare GET to one of the providers below. No scan
// target, no result, no device name, no MAC address and no note is ever
// included, because none of it is passed to this module in the first place.

/**
 * The providers, in the order they are tried.
 *
 * Both are long-standing, single-purpose "what is my address" endpoints that
 * answer with the address and nothing else. Every host named here must also
 * appear in `connect-src` in `src-tauri/tauri.conf.json`, or the packaged app
 * cannot reach it; `scripts/verify-csp.mjs` fails the build when the two lists
 * drift apart.
 */
export const PUBLIC_IP_PROVIDERS = [
  { name: "ipify", url: "https://api64.ipify.org?format=json", json: true },
  { name: "icanhazip", url: "https://icanhazip.com", json: false },
] as const;

/** How long the whole lookup may take before it is abandoned. */
export const PUBLIC_IP_TIMEOUT_MS = 8_000;

/**
 * A lookup that failed against every provider.
 *
 * `message` is what a person is shown. `technical` is the per-provider detail
 * behind the "Technical details" disclosure, which is the difference between
 * "it did not work" and "your DNS is refusing api64.ipify.org".
 */
export class PublicIpError extends Error {
  readonly technical: string;

  constructor(message: string, technical: string) {
    super(message);
    this.name = "PublicIpError";
    this.technical = technical;
  }
}

/**
 * A conservative shape check on whatever the provider returned.
 *
 * The value is rendered as an address and offered for copying, so a provider
 * that answers with an HTML error page, a rate-limit notice or an empty body
 * must be treated as a failure and the next provider tried, rather than
 * displayed as though it were an address.
 */
export function looksLikeIp(value: string): boolean {
  if (value.length === 0 || value.length > 45) return false;
  if (/^(\d{1,3}\.){3}\d{1,3}$/.test(value)) {
    return value.split(".").every((part) => Number(part) <= 255);
  }
  // IPv6, including the IPv4-mapped forms api64.ipify.org can return.
  return /:/.test(value) && /^[0-9a-fA-F:.]+$/.test(value) && !/[.:]{3}/.test(value);
}

/**
 * Ask each provider in turn for this machine's public address.
 *
 * `fetchImpl` is injected so the browser demo can drive the real fallback
 * logic against scripted providers, and so the tests can cover a first-provider
 * failure without a network.
 *
 * Aborting `signal` cancels the lookup immediately and rethrows the
 * `AbortError`: a cancelled lookup is a person changing their mind or a
 * timeout, never a provider fault, and it must not be reported as one.
 */
export async function lookupPublicIp(
  fetchImpl: typeof fetch,
  signal?: AbortSignal,
): Promise<string> {
  const failures: string[] = [];

  for (const provider of PUBLIC_IP_PROVIDERS) {
    if (signal?.aborted) throw abortError();
    try {
      const response = await fetchImpl(provider.url, {
        signal,
        // Nothing about this request should be reused or attributed: no cookies
        // on the way out, no cached answer standing in for a fresh check.
        cache: "no-store",
        credentials: "omit",
        redirect: "follow",
      });
      if (!response.ok) {
        failures.push(`${provider.name}: HTTP ${response.status}`);
        continue;
      }
      const value = provider.json
        ? String(((await response.json()) as { ip?: unknown }).ip ?? "").trim()
        : (await response.text()).trim();
      if (looksLikeIp(value)) return value;
      failures.push(`${provider.name}: unrecognised response`);
    } catch (error) {
      // An aborted request is the operator navigating away or the timeout
      // firing, not a failure to fall back from.
      if (isAbortError(error)) throw error;
      failures.push(`${provider.name}: ${error instanceof Error ? error.message : String(error)}`);
    }
  }

  throw new PublicIpError(
    "No public-IP provider answered.",
    failures.join("\n") || "No provider was reachable.",
  );
}

export function isAbortError(error: unknown): boolean {
  return (
    (typeof DOMException !== "undefined" &&
      error instanceof DOMException &&
      error.name === "AbortError") ||
    (error instanceof Error && error.name === "AbortError")
  );
}

export function abortError(): Error {
  const error = new Error("The lookup was cancelled.");
  error.name = "AbortError";
  return error;
}
