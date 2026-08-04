import { describe, expect, it, vi } from "vitest";
import {
  PUBLIC_IP_PROVIDERS,
  PublicIpError,
  abortError,
  isAbortError,
  looksLikeIp,
  lookupPublicIp,
} from "./publicIp";

const [PRIMARY, FALLBACK] = PUBLIC_IP_PROVIDERS;

/** A documentation address (RFC 5737); never anyone's real one. */
const IP = "203.0.113.24";
const OTHER_IP = "198.51.100.17";

const json = (ip: string) =>
  new Response(JSON.stringify({ ip }), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
const text = (body: string) =>
  new Response(body, { status: 200, headers: { "content-type": "text/plain" } });

describe("looksLikeIp", () => {
  it("accepts the shapes a provider actually returns", () => {
    expect(looksLikeIp("203.0.113.24")).toBe(true);
    expect(looksLikeIp("8.8.8.8")).toBe(true);
    expect(looksLikeIp("2001:db8::1")).toBe(true);
    expect(looksLikeIp("::ffff:203.0.113.24")).toBe(true);
  });

  it("rejects anything that would be wrong to display as an address", () => {
    // A provider answering with an error page, a rate-limit notice or nothing
    // must not be rendered as though it were an address.
    expect(looksLikeIp("")).toBe(false);
    expect(looksLikeIp("<!DOCTYPE html>")).toBe(false);
    expect(looksLikeIp("rate limit exceeded")).toBe(false);
    expect(looksLikeIp("999.1.1.1")).toBe(false);
    expect(looksLikeIp("203.0.113")).toBe(false);
    expect(looksLikeIp("x".repeat(60))).toBe(false);
  });
});

describe("lookupPublicIp", () => {
  it("returns the first provider's answer without contacting the second", async () => {
    const fetchImpl = vi.fn(async (_url: string, _init?: RequestInit) => json(IP));

    expect(await lookupPublicIp(fetchImpl as unknown as typeof fetch)).toBe(IP);
    expect(fetchImpl).toHaveBeenCalledTimes(1);
    expect(fetchImpl.mock.calls[0][0]).toBe(PRIMARY.url);
  });

  it("sends no credentials and accepts no cached answer", async () => {
    const fetchImpl = vi.fn(async (_url: string, _init?: RequestInit) => json(IP));
    await lookupPublicIp(fetchImpl as unknown as typeof fetch);

    const init = fetchImpl.mock.calls[0][1] as RequestInit;
    expect(init.credentials).toBe("omit");
    expect(init.cache).toBe("no-store");
  });

  it("falls back to the second provider when the first returns an error status", async () => {
    const fetchImpl = vi.fn(async (url: string) =>
      url === PRIMARY.url ? new Response("upstream error", { status: 503 }) : text(`${OTHER_IP}\n`),
    );

    expect(await lookupPublicIp(fetchImpl as unknown as typeof fetch)).toBe(OTHER_IP);
    expect(fetchImpl).toHaveBeenCalledTimes(2);
    expect(fetchImpl.mock.calls[1][0]).toBe(FALLBACK.url);
  });

  it("falls back when the first provider is unreachable", async () => {
    const fetchImpl = vi.fn(async (url: string) => {
      if (url === PRIMARY.url) throw new TypeError("Failed to fetch");
      return text(OTHER_IP);
    });

    expect(await lookupPublicIp(fetchImpl as unknown as typeof fetch)).toBe(OTHER_IP);
  });

  it("falls back when the first provider answers with something that is not an address", async () => {
    const fetchImpl = vi.fn(async (url: string) =>
      url === PRIMARY.url ? json("" as unknown as string) : text(OTHER_IP),
    );

    expect(await lookupPublicIp(fetchImpl as unknown as typeof fetch)).toBe(OTHER_IP);
  });

  it("reports every provider's failure once none of them answered", async () => {
    const fetchImpl = vi.fn(async (url: string) => {
      if (url === PRIMARY.url) return new Response("nope", { status: 500 });
      throw new TypeError("Failed to fetch");
    });

    const error = await lookupPublicIp(fetchImpl as unknown as typeof fetch).catch((e) => e);
    expect(error).toBeInstanceOf(PublicIpError);
    // The person sees the message; the technical detail says which provider
    // failed and how, which is the difference between "it broke" and "your DNS
    // is refusing api64.ipify.org".
    expect(error.message).toBe("No public-IP provider answered.");
    expect(error.technical).toContain("ipify: HTTP 500");
    expect(error.technical).toContain("icanhazip: Failed to fetch");
  });

  it("rethrows an abort instead of treating it as a provider failure", async () => {
    const controller = new AbortController();
    const fetchImpl = vi.fn(async () => {
      controller.abort();
      throw abortError();
    });

    const error = await lookupPublicIp(
      fetchImpl as unknown as typeof fetch,
      controller.signal,
    ).catch((e) => e);

    expect(isAbortError(error)).toBe(true);
    expect(error).not.toBeInstanceOf(PublicIpError);
    // A cancelled first provider must not silently fall through to the second.
    expect(fetchImpl).toHaveBeenCalledTimes(1);
  });

  it("makes no request at all when the signal is already aborted", async () => {
    const controller = new AbortController();
    controller.abort();
    const fetchImpl = vi.fn(async (_url: string) => json(IP));

    await expect(
      lookupPublicIp(fetchImpl as unknown as typeof fetch, controller.signal),
    ).rejects.toSatisfy(isAbortError);
    expect(fetchImpl).not.toHaveBeenCalled();
  });

  it("only ever contacts the declared providers", async () => {
    const seen: string[] = [];
    const fetchImpl = vi.fn(async (url: string) => {
      seen.push(url);
      throw new TypeError("Failed to fetch");
    });

    await lookupPublicIp(fetchImpl as unknown as typeof fetch).catch(() => {});
    expect(seen).toEqual(PUBLIC_IP_PROVIDERS.map((p) => p.url));
    for (const url of seen) expect(new URL(url).protocol).toBe("https:");
  });
});
