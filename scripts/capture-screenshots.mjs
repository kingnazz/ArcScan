#!/usr/bin/env node
// Capture the product screenshots the website uses.
//
// Every shot comes from the real v1.7 interface driven in a browser against the
// built-in demo network, so the images can never show an older UI. The network is
// entirely fictional, so no real client, hostname, MAC address or public address
// is ever in a published image.
//
// Run it the same way as scripts/verify-ui.mjs:
//
//   npm run build
//   npm run preview &
//   npm i --no-save playwright sharp
//   node scripts/capture-screenshots.mjs
//
// PNGs land in site/assets/shots/ and are converted to WebP when sharp is
// available. Set PLAYWRIGHT_CHROMIUM_PATH to reuse a Chromium already installed.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { chromium } from "playwright";

const URL = process.env.ARCSCAN_URL ?? "http://localhost:4173/";
const OUT = process.env.ARCSCAN_SHOTS ?? "site/assets/shots";
/** 2x, so the images stay crisp on Retina and 150% Windows scaling. */
const SCALE = 2;
const WIDE = { width: 1440, height: 900 };
const NARROW = { width: 940, height: 700 };

mkdirSync(OUT, { recursive: true });

const executablePath = process.env.PLAYWRIGHT_CHROMIUM_PATH;
const browser = await chromium.launch(executablePath ? { executablePath } : {});

/** Reduced motion, so no shot catches a half-finished transition. */
const context = await browser.newContext({
  viewport: WIDE,
  deviceScaleFactor: SCALE,
  reducedMotion: "reduce",
});
const page = await context.newPage();

const shots = [];
async function shot(name) {
  const file = join(OUT, `${name}.png`);
  await page.screenshot({ path: file });
  shots.push(file);
  console.log(`  ${file}`);
}

async function setTheme(theme) {
  await page.evaluate((value) => {
    const raw = localStorage.getItem("arcscan-settings");
    const settings = raw ? JSON.parse(raw) : {};
    settings.theme = value;
    localStorage.setItem("arcscan-settings", JSON.stringify(settings));
    localStorage.setItem("arcscan-theme", value);
  }, theme);
  await page.reload({ waitUntil: "networkidle" });
  await page.waitForTimeout(250);
}

async function runScan() {
  await page.locator("#scan-target").fill("192.168.10.0/24");
  await page.getByRole("button", { name: "Scan", exact: true }).click();
}

async function waitForScanEnd() {
  await page.getByRole("button", { name: "Stop" }).waitFor({ state: "detached", timeout: 30_000 });
  await page.waitForTimeout(800);
}

console.log("Capturing ArcScan v1.7 screenshots");

// --- Dark theme -----------------------------------------------------------
await page.goto(URL, { waitUntil: "networkidle" });
await setTheme("dark");

await shot("empty-dark");

await runScan();
// Part-way through, so the shot genuinely shows results streaming in.
await page.locator("tbody tr").nth(4).waitFor({ timeout: 10_000 });
await page.waitForTimeout(200);
await shot("scanning-dark");

await waitForScanEnd();
await shot("results-dark");

// Device drawer, on a device with a web interface and a change to show.
await page.locator("tbody tr", { hasText: "Front Office Printer" }).dblclick();
await page.getByRole("complementary").waitFor({ timeout: 5_000 });
await page.waitForTimeout(250);
await shot("device-dark");
await page.keyboard.press("Escape");
await page.waitForTimeout(150);

await page.locator("header nav button", { hasText: "Changes" }).click();
await page.waitForTimeout(300);
await shot("changes-dark");

await page.locator("header nav button", { hasText: "History" }).click();
await page.waitForTimeout(300);
await shot("history-dark");

await page.getByRole("button", { name: "Settings" }).click();
await page.waitForTimeout(300);
await shot("settings-dark");
await page.keyboard.press("Escape");

// Narrow window, to show the layout holding together.
await page.locator("header nav button", { hasText: "Devices" }).click();
await page.setViewportSize(NARROW);
await page.waitForTimeout(300);
await shot("narrow-dark");
await page.setViewportSize(WIDE);

// --- Light theme ----------------------------------------------------------
await setTheme("light");
await runScan();
await waitForScanEnd();
await shot("results-light");

await page.locator("tbody tr", { hasText: "Backup NAS" }).dblclick();
await page.getByRole("complementary").waitFor({ timeout: 5_000 });
await page.waitForTimeout(250);
await shot("device-light");
await page.keyboard.press("Escape");

await page.locator("header nav button", { hasText: "Changes" }).click();
await page.waitForTimeout(300);
await shot("changes-light");

await page.locator("header nav button", { hasText: "History" }).click();
await page.waitForTimeout(300);
await shot("history-light");

await browser.close();

// --- Optimise -------------------------------------------------------------
// WebP at 2x is roughly a third the size of PNG at the same visible quality,
// and the PNGs stay as the fallback for anything that cannot decode it.
try {
  const sharp = (await import("sharp")).default;
  console.log("Writing WebP versions");
  for (const file of shots) {
    const webp = file.replace(/\.png$/, ".webp");
    const info = await sharp(file).webp({ quality: 82, effort: 6 }).toFile(webp);
    console.log(`  ${webp} (${Math.round(info.size / 1024)} kB)`);
  }
} catch {
  console.log("sharp is not installed, so only PNGs were written.");
  console.log("Install it with `npm i --no-save sharp` and re-run to add WebP.");
}

console.log(`\nDone: ${shots.length} screenshots in ${OUT}`);
