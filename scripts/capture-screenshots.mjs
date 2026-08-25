#!/usr/bin/env node
// Capture the product screenshots the website uses.
//
// Every shot comes from the real v1.8.3 interface driven in a browser against the
// built-in demo network, so the images can never show an older UI. The networks
// are entirely fictional, so no real client, hostname, MAC address or public
// address is ever in a published image.
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

const nav = (label) => page.locator("header nav button", { hasText: label });

async function setTheme(theme) {
  await page.evaluate((value) => {
    const raw = localStorage.getItem("arcscan-settings");
    const settings = raw ? JSON.parse(raw) : {};
    settings.theme = value;
    localStorage.setItem("arcscan-settings", JSON.stringify(settings));
    localStorage.setItem("arcscan-theme", value);
  }, theme);
  await page.reload({ waitUntil: "networkidle" });
  await page.waitForTimeout(300);
}

async function runScan() {
  await page.locator("#scan-target").fill("192.168.1.0/24");
  await page.locator('form button[type="submit"]').click();
}

async function waitForScanEnd() {
  await page.getByRole("button", { name: "Stop" }).waitFor({ state: "detached", timeout: 30_000 });
  await page.waitForTimeout(900);
}

console.log("Capturing ArcScan v1.8.2 screenshots");

// --- Dark theme -----------------------------------------------------------
await page.goto(URL, { waitUntil: "networkidle" });
await setTheme("dark");

await shot("empty-dark");

// The populated Inventory, which is what the release is about.
await nav("Inventory").click();
await page.locator("tbody tr").first().waitFor({ timeout: 10_000 });
await page.waitForTimeout(400);
await shot("inventory-dark");

// The same view filtered to the devices that stopped answering.
await page.getByLabel("Filter the inventory").selectOption("missing");
await page.waitForTimeout(400);
await shot("inventory-missing-dark");
await page.getByLabel("Filter the inventory").selectOption("all");
await page.waitForTimeout(300);

// The Inventory with the device types discovery established, which is what
// v1.8.2 is about. The Type column is optional, so it has to be turned on the
// same way an operator would.
await page.getByRole("button", { name: "Settings" }).click();
{
  const panel = page.getByRole("complementary", { name: "Settings" });
  for (const key of ["type", "model"]) {
    await panel.locator(`#settings-inventory-column-${key}`).click();
    await page.waitForTimeout(120);
  }
}
await page.keyboard.press("Escape");
await page.waitForTimeout(300);
await nav("Inventory").click();
await page.locator("tbody tr").first().waitFor({ timeout: 10_000 });
await page.waitForTimeout(400);
await shot("inventory-types-dark");
// Put the table back the way the other shots expect it.
await page.getByRole("button", { name: "Settings" }).click();
{
  const panel = page.getByRole("complementary", { name: "Settings" });
  for (const key of ["type", "model"]) {
    await panel.locator(`#settings-inventory-column-${key}`).click();
    await page.waitForTimeout(120);
  }
}
await page.keyboard.press("Escape");
await page.waitForTimeout(300);

// Two recognised networks with friendly names, sorted so the grouping is
// obvious. A native select cannot be screenshotted open, and would show the
// operating system's menu rather than ArcScan's anyway.
await page.getByRole("columnheader", { name: "Network" }).getByRole("button").click();
await page.waitForTimeout(400);
await shot("inventory-networks-dark");
await page.getByRole("columnheader", { name: "Last seen" }).getByRole("button").click();
await page.waitForTimeout(300);

// The Changes inbox.
await nav("Changes").click();
await page.locator("main ul > li").first().waitFor({ timeout: 10_000 });
await page.waitForTimeout(400);
await shot("changes-dark");

// A change opened in the device drawer, which is where a change is reviewed.
// Scrolled to the recorded changes, since that is the point of the shot.
await page.locator("main ul > li").first().getByRole("button", { name: "Review" }).click();
const drawer = page.getByRole("complementary");
await drawer.waitFor({ timeout: 5_000 });
await drawer.getByText("Recorded changes").scrollIntoViewIfNeeded();
await page.waitForTimeout(500);
await shot("change-detail-dark");
await page.keyboard.press("Escape");
await page.waitForTimeout(250);

// --- Scanning -------------------------------------------------------------
await nav("Scan").click();
await page.waitForTimeout(250);
await runScan();
// Part-way through, so the shot genuinely shows results streaming in.
await page.locator("tbody tr").nth(3).waitFor({ timeout: 15_000 });
await page.waitForTimeout(200);
await shot("scanning-dark");

await waitForScanEnd();
await shot("results-dark");

// The optional public-IP lookup, after it has been asked. The demo answers from
// scripted providers with an address reserved for documentation by RFC 5737, so
// a published image can never contain a real one.
const contextRow = page.locator("header + div").first();
await contextRow.getByRole("button", { name: "Check public IP" }).click();
await contextRow
  .getByRole("button", { name: "Check the public IP again" })
  .waitFor({ timeout: 12_000 });
// Away from the controls, so no shot catches a hover state.
await page.mouse.move(4, 880);
await page.waitForTimeout(400);
await shot("public-ip-dark");

// The device drawer from the scan results.
await page.locator("tbody tr", { hasText: "Office Printer" }).first().dblclick();
await page.getByRole("complementary").waitFor({ timeout: 5_000 });
await page.waitForTimeout(300);
await shot("device-dark");

// The same drawer, scrolled to the Discovery section: the detected name, the
// device type with its confidence, and the evidence behind it.
{
  const drawer = page.getByRole("complementary").last();
  await drawer.getByText("Discovery", { exact: true }).first().scrollIntoViewIfNeeded();
  await page.waitForTimeout(400);
  await shot("device-discovery-dark");
}
await page.keyboard.press("Escape");
await page.waitForTimeout(200);

// v1.8.3: the same section on a device whose type the operator has corrected.
// ArcScan reads the television as a media device; the drawer shows the
// correction, says who made it, and keeps ArcScan's own answer underneath.
{
  await nav("Inventory").click();
  await page.waitForTimeout(300);
  await page.locator('tbody tr:has-text("192.168.1.44")').first().dblclick();
  await page.getByRole("complementary").waitFor({ timeout: 5_000 });
  const drawer = page.getByRole("complementary").last();
  await drawer.getByText("Discovery", { exact: true }).first().scrollIntoViewIfNeeded();
  await page.waitForTimeout(400);
  await shot("device-type-override-dark");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
}

// v1.8.3: a device whose evidence has gone stale. Everything it once
// advertised is still on file and still dated, in discovery scans rather than
// in days, and its confidence has been reduced because nothing has confirmed
// it in three scans that could have.
{
  await nav("Inventory").click();
  await page.waitForTimeout(300);
  await page.locator('tbody tr:has-text("192.168.1.81")').first().dblclick();
  await page.getByRole("complementary").waitFor({ timeout: 5_000 });
  const drawer = page.getByRole("complementary").last();
  await drawer.getByText("Discovery", { exact: true }).first().scrollIntoViewIfNeeded();
  await page.waitForTimeout(400);
  await shot("device-stale-evidence-dark");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
}

await nav("History").click();
await page.waitForTimeout(400);
await shot("history-dark");

await page.getByRole("button", { name: "Settings" }).click();
await page.waitForTimeout(400);
await shot("settings-dark");

// The Networks section, scrolled into view.
const settings = page.getByRole("complementary", { name: "Settings" });
await settings.getByLabel(/^Name for /).first().scrollIntoViewIfNeeded();
await page.waitForTimeout(350);
await shot("settings-networks-dark");
await page.keyboard.press("Escape");
await page.waitForTimeout(200);

// --- Partial-scan states --------------------------------------------------
// A genuinely stopped scan, not a mocked-up screen: the scan is started and
// then cancelled part-way, exactly as pressing Stop does.
await nav("Scan").click();
await page.waitForTimeout(250);
await runScan();
await page.locator("tbody tr").nth(3).waitFor({ timeout: 15_000 });
await page.waitForTimeout(250);
await page.getByRole("button", { name: /^Stop/ }).click();
await page.getByRole("button", { name: /^Stop/ }).waitFor({ state: "detached", timeout: 15_000 });
await page.waitForTimeout(1_000);

await nav("History").click();
await page.waitForTimeout(500);
await shot("history-partial-dark");

// The comparison for that partial scan, which explains why it has none.
await page.locator("main ul > li").first().locator("button").first().click();
await page.waitForTimeout(600);
await page.getByRole("button", { name: "Why no comparison?" }).click();
await page.waitForTimeout(500);
await shot("changes-partial-dark");

// Narrow window, to show the layout holding together where it matters most.
await nav("Inventory").click();
await page.setViewportSize(NARROW);
await page.waitForTimeout(400);
await shot("narrow-dark");
await page.setViewportSize(WIDE);

// --- Light theme ----------------------------------------------------------
await setTheme("light");

await nav("Inventory").click();
await page.locator("tbody tr").first().waitFor({ timeout: 10_000 });
await page.waitForTimeout(400);
await shot("inventory-light");

await nav("Changes").click();
await page.locator("main ul > li").first().waitFor({ timeout: 10_000 });
await page.waitForTimeout(400);
await shot("changes-light");

await nav("Scan").click();
await page.waitForTimeout(250);
await runScan();
await waitForScanEnd();
await shot("results-light");

await page.locator("tbody tr", { hasText: "Home NAS" }).first().dblclick();
await page.getByRole("complementary").waitFor({ timeout: 5_000 });
await page.waitForTimeout(300);
await shot("device-light");
await page.keyboard.press("Escape");

await nav("History").click();
await page.waitForTimeout(400);
await shot("history-light");

await browser.close();

// --- Optimise -------------------------------------------------------------
// WebP at 2x is roughly a third the size of PNG at the same visible quality,
// and the PNGs stay as the fallback for anything that cannot decode it.
try {
  const sharp = (await import("sharp")).default;
  console.log("Writing WebP versions");
  for (const file of shots) {
    // Three widths, so a phone is not sent a 2,880px screenshot. The browser
    // picks from the srcset: 800 for phones, 1440 for ordinary desktops, and
    // the full capture only for high-density screens.
    const variants = [
      { suffix: "-800.webp", width: 800, quality: 78 },
      { suffix: ".webp", width: 1440, quality: 80 },
      { suffix: "@2x.webp", width: null, quality: 82 },
    ];
    const written = [];
    for (const variant of variants) {
      const out = file.replace(/\.png$/, variant.suffix);
      const pipeline = sharp(file);
      if (variant.width) pipeline.resize({ width: variant.width });
      const info = await pipeline.webp({ quality: variant.quality, effort: 6 }).toFile(out);
      written.push(`${variant.suffix} ${Math.round(info.size / 1024)} kB`);
    }
    console.log(`  ${file.replace(/\.png$/, "")}: ${written.join(", ")}`);
  }
} catch {
  console.log("sharp is not installed, so only PNGs were written.");
  console.log("Install it with `npm i --no-save sharp` and re-run to add WebP.");
}

console.log(`\nDone: ${shots.length} screenshots in ${OUT}`);
