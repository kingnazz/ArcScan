#!/usr/bin/env node
// Verify the built application actually runs under the Content Security Policy
// declared in tauri.conf.json.
//
// A CSP that is too strict is invisible in review and in the browser preview,
// which serves no policy at all: the app simply breaks — or silently loses a
// feature — once it is packaged. This loads the real built assets with the real
// production policy applied as a response header, exercises the parts of the
// app that a policy can break (module scripts, stylesheets, the theme script
// that runs before first paint, fonts, images, a streaming scan and an export),
// and fails on any CSP violation.
//
// It cannot cover Tauri's IPC, which does not exist in a plain browser; the
// policy's ipc: and http://ipc.localhost sources still need one run of the
// packaged app. Everything else is checked here.
//
//   npm run build
//   npm run preview &
//   npm i --no-save playwright
//   node scripts/verify-csp.mjs
//
// Set PLAYWRIGHT_CHROMIUM_PATH to reuse a Chromium already on the machine.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { chromium } from "playwright";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const URL_BASE = process.env.ARCSCAN_URL ?? "http://localhost:4173/";

const config = JSON.parse(readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"));
const csp = config.app?.security?.csp;
if (!csp || typeof csp !== "object") {
  console.log("FAIL  a production CSP is declared in tauri.conf.json");
  process.exit(1);
}

// Serialize the object form Tauri accepts into a policy header.
const policy = Object.entries(csp)
  .map(([directive, value]) => `${directive} ${Array.isArray(value) ? value.join(" ") : value}`)
  .join("; ");

console.log(`Policy under test:\n  ${policy.replace(/; /g, "\n  ")}\n`);

const violations = [];
const errors = [];

const staticStep = (name, fn) => {
  try {
    const detail = fn();
    console.log(`PASS  ${name}${detail ? ` — ${detail}` : ""}`);
  } catch (e) {
    console.log(`FAIL  ${name} — ${e.message}`);
    process.exitCode = 1;
  }
};

// ---------------------------------------------------------------------------
// Static checks on the policy itself. A browser can only prove that what the
// app does today is allowed; these prove that what the policy allows is still
// narrow, which is the half that rots silently.
// ---------------------------------------------------------------------------

const devCsp = config.app?.security?.devCsp ?? {};
const sources = (directive, policyObject) => {
  const value = policyObject[directive];
  if (value == null) return [];
  return (Array.isArray(value) ? value.join(" ") : value).split(/\s+/).filter(Boolean);
};

staticStep("no wildcard or plaintext origin is allowed anywhere", () => {
  for (const [label, policyObject] of [
    ["production", csp],
    ["development", devCsp],
  ]) {
    for (const [directive, value] of Object.entries(policyObject)) {
      for (const source of sources(directive, policyObject)) {
        if (source === "*" || source.startsWith("*.") || source.includes("://*")) {
          throw new Error(`${label} ${directive} allows the wildcard ${source}`);
        }
        // http: is only ever acceptable for Tauri's own IPC origin and, in
        // development, the local Vite server.
        const localhost = /^(ws|http):\/\/(localhost|127\.0\.0\.1)(:\d+)?$/.test(source);
        const ipc = source === "ipc:" || source === "http://ipc.localhost";
        if (/^https?:\/\//.test(source) && !source.startsWith("https://") && !localhost && !ipc) {
          throw new Error(`${label} ${directive} allows plaintext ${source}`);
        }
        if (label === "production" && localhost) {
          throw new Error(`production ${directive} allows the dev server ${source}`);
        }
      }
      void value;
    }
  }
  return "no wildcards, no plaintext beyond Tauri IPC";
});

staticStep("no script source can execute code from a string or a third party", () => {
  const script = sources("script-src", csp);
  const allowed = new Set(["'self'"]);
  const extra = script.filter((s) => !allowed.has(s));
  if (extra.length > 0) throw new Error(`production script-src also allows ${extra.join(", ")}`);
  for (const [label, policyObject] of [
    ["production", csp],
    ["development", devCsp],
  ]) {
    for (const directive of ["script-src", "default-src", "style-src", "connect-src"]) {
      if (sources(directive, policyObject).includes("'unsafe-eval'")) {
        throw new Error(`${label} ${directive} allows 'unsafe-eval'`);
      }
    }
  }
  if (sources("object-src", csp)[0] !== "'none'") throw new Error("object-src is not 'none'");
  if (sources("frame-src", csp)[0] !== "'none'") throw new Error("frame-src is not 'none'");
  return "script-src 'self', no eval, no frames, no objects";
});

staticStep("Tauri's own IPC channel is still reachable", () => {
  const connect = sources("connect-src", csp);
  for (const needed of ["'self'", "ipc:", "http://ipc.localhost"]) {
    if (!connect.includes(needed)) throw new Error(`connect-src has lost ${needed}`);
  }
  return "ipc: and http://ipc.localhost present";
});

staticStep("connect-src allows the public-IP providers and nothing else", () => {
  // The provider list and the policy are edited in different files, so a new
  // provider that nobody allowlisted would work in the browser demo and fail
  // only once packaged. This is the check that stops that shipping.
  const module = readFileSync(join(root, "src/lib/publicIp.ts"), "utf8");
  const declared = [...module.matchAll(/url:\s*"(https:\/\/[^"]+)"/g)].map(
    (m) => new URL(m[1]).origin,
  );
  if (declared.length === 0) throw new Error("no providers found in src/lib/publicIp.ts");

  for (const [label, policyObject] of [
    ["production", csp],
    ["development", devCsp],
  ]) {
    const connect = sources("connect-src", policyObject);
    for (const origin of declared) {
      if (!connect.includes(origin)) {
        throw new Error(`${label} connect-src does not allow the provider ${origin}`);
      }
    }
    // And nothing beyond them: every remaining remote origin must be a provider.
    const remote = connect.filter(
      (s) => s.startsWith("https://") && !s.includes("ipc.localhost"),
    );
    const extra = remote.filter((s) => !declared.includes(s));
    if (extra.length > 0) {
      throw new Error(`${label} connect-src allows ${extra.join(", ")}, which is not a provider`);
    }
  }
  return declared.join(", ");
});


const executablePath = process.env.PLAYWRIGHT_CHROMIUM_PATH;
const browser = await chromium.launch(executablePath ? { executablePath } : {});
const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });

// Apply the policy to the document exactly as the packaged app would receive
// it, leaving the assets themselves untouched.
await context.route("**/*", async (route) => {
  const response = await route.fetch();
  const headers = { ...response.headers() };
  const type = headers["content-type"] ?? "";
  if (type.includes("text/html")) headers["content-security-policy"] = policy;
  await route.fulfill({ response, headers });
});

const page = await context.newPage();
// Record the document's classes at the moment parsing finishes: the classic
// theme script has run by then, and deferred module scripts (React) have not,
// so this observes the pre-paint state rather than racing the app's mount.
await page.addInitScript(() => {
  window.__classesBeforeMount = null;
  document.addEventListener("readystatechange", () => {
    if (window.__classesBeforeMount === null && document.readyState === "interactive") {
      window.__classesBeforeMount = document.documentElement.className;
    }
  });
});
page.on("console", (message) => {
  const text = message.text();
  if (/Content Security Policy|Refused to/i.test(text)) violations.push(text);
  else if (message.type() === "error") errors.push(text);
});
page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));

const step = async (name, fn) => {
  try {
    const detail = await fn();
    console.log(`PASS  ${name}${detail ? ` — ${detail}` : ""}`);
  } catch (e) {
    console.log(`FAIL  ${name} — ${e.message}`);
    process.exitCode = 1;
  }
};

await step("the application loads and mounts under the policy", async () => {
  await page.goto(URL_BASE, { waitUntil: "networkidle" });
  await page.getByRole("heading", { name: "Scan a network" }).waitFor({ timeout: 10000 });
  const styled = await page.evaluate(() => {
    const header = document.querySelector("header");
    return header ? getComputedStyle(header).borderBottomStyle : "none";
  });
  // A blocked stylesheet leaves every computed style at its initial value.
  if (styled === "none") throw new Error("stylesheet did not apply; style-src is too strict");
  return "React mounted, stylesheet applied";
});

await step("the pre-paint theme script runs", async () => {
  // This script used to be inline; under script-src 'self' it must still run,
  // or the window flashes the wrong theme on every launch. Checked before the
  // app mounts, so a passing result cannot come from React applying the theme.
  await page.evaluate(() => localStorage.setItem("arcscan-theme", "dark"));
  await page.reload({ waitUntil: "networkidle" });
  const before = await page.evaluate(() => window.__classesBeforeMount);
  if (before === null) throw new Error("could not observe the pre-mount state");
  if (!/\bdark\b/.test(before)) {
    throw new Error(`the theme script did not run before paint (classes: "${before}")`);
  }
  await page.evaluate(() => localStorage.setItem("arcscan-theme", "light"));
  await page.reload({ waitUntil: "networkidle" });
  return "dark applied before the app mounted";
});

await step("a scan streams and completes under the policy", async () => {
  await page.getByRole("button", { name: /^Scan 192\.168\.1\.0\/24$/ }).click();
  await page.locator("tbody tr").first().waitFor({ timeout: 8000 });
  await page.getByRole("button", { name: /^Stop/ }).waitFor({ state: "detached", timeout: 40000 });
  await page.waitForTimeout(600);
  const rows = await page.locator("tbody tr").count();
  if (rows === 0) throw new Error("no devices after the scan");
  return `${rows} devices`;
});

await step("a scan export is produced under the policy", async () => {
  // In the browser the export goes through a blob URL, which some policies
  // block outright.
  const download = page.waitForEvent("download", { timeout: 10000 });
  await page.getByRole("button", { name: "Export", exact: true }).click();
  await page.getByRole("button", { name: "CSV spreadsheet" }).click();
  const file = await download;
  return file.suggestedFilename();
});

await step("an inventory export is produced under the policy", async () => {
  await page.locator("header nav button", { hasText: "Inventory" }).click();
  await page.locator("tbody tr").first().waitFor({ timeout: 5000 });
  const download = page.waitForEvent("download", { timeout: 10000 });
  await page.getByRole("button", { name: "Export", exact: true }).click();
  await page.getByRole("button", { name: "CSV spreadsheet" }).click();
  const file = await download;
  return file.suggestedFilename();
});

await step("a changes export is produced under the policy", async () => {
  await page.locator("header nav button", { hasText: "Changes" }).click();
  await page.getByLabel("Filter changes", { exact: true }).selectOption("all");
  await page.locator("main ul > li").first().waitFor({ timeout: 5000 });
  const download = page.waitForEvent("download", { timeout: 10000 });
  await page.getByRole("button", { name: "Export", exact: true }).click();
  await page.getByRole("button", { name: "CSV spreadsheet" }).click();
  const file = await download;
  await page.locator("header nav button", { hasText: "Scan" }).click();
  return file.suggestedFilename();
});

await step("both themes render under the policy", async () => {
  for (const label of [/Switch to the dark theme/, /Switch to the light theme/]) {
    await page.getByRole("button", { name: label }).click();
    await page.waitForTimeout(250);
  }
  return "toggled dark and light";
});

await step("the device panel and settings open under the policy", async () => {
  await page.locator("tbody tr").first().dblclick();
  await page.getByRole("complementary").waitFor({ timeout: 5000 });
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("complementary", { name: "Settings" }).waitFor({ timeout: 5000 });
  await page.keyboard.press("Escape");
});

if (violations.length > 0) {
  console.log(`\nFAIL  no CSP violations — ${violations.length}:`);
  for (const v of violations.slice(0, 10)) console.log(`      ${v}`);
  process.exitCode = 1;
} else {
  console.log("PASS  no CSP violations");
}

if (errors.length > 0) {
  console.log(`\nFAIL  console clean — ${errors.length} error(s):`);
  for (const e of errors.slice(0, 8)) console.log(`      ${e}`);
  process.exitCode = 1;
} else {
  console.log("PASS  console clean");
}

await browser.close();
