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
  await page.getByRole("button", { name: /^Scan 192\.168\.10\.0\/24$/ }).click();
  await page.locator("tbody tr").first().waitFor({ timeout: 8000 });
  await page.getByRole("button", { name: /^Stop/ }).waitFor({ state: "detached", timeout: 40000 });
  await page.waitForTimeout(600);
  const rows = await page.locator("tbody tr").count();
  if (rows === 0) throw new Error("no devices after the scan");
  return `${rows} devices`;
});

await step("an export is produced under the policy", async () => {
  // In the browser the export goes through a blob URL, which some policies
  // block outright.
  const download = page.waitForEvent("download", { timeout: 10000 });
  await page.getByRole("button", { name: "Export", exact: true }).click();
  await page.getByRole("button", { name: "CSV spreadsheet" }).click();
  const file = await download;
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
