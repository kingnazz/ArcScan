#!/usr/bin/env node

import { readFileSync, writeFileSync } from "node:fs";

const path = "scripts/verify-site.mjs";
let s = readFileSync(path, "utf8");

function replaceRequired(before, after, label) {
  if (s.includes(after)) return;
  if (!s.includes(before)) throw new Error(`Could not find ${label}`);
  s = s.replace(before, after);
}

const releaseStart = 'await step("the release section states the 1.8.4 improvements", async () => {';
const releaseEnd = '\n});\n\nawait step("the What changed link opens the local 1.8.4 page", async () => {';
const start = s.indexOf(releaseStart);
const end = s.indexOf(releaseEnd, start);
if (start < 0 || end < 0) throw new Error("Could not locate current release assertion block");

const releaseBlock = `await step("the release section states the 1.8.5 improvements", async () => {
  const section = page.locator("#whats-new");
  await section.waitFor({ timeout: 3000 });
  const text = (await section.innerText()).toLowerCase();

  if (!text.includes("1.8.5")) throw new Error("the section does not name the version");

  const claims = [
    [/arcatlas/, "the ArcAtlas handoff"],
    [/one selected network|choose a network/, "one-network scope"],
    [/explicit|deliberate send|confirm/, "explicit operator action"],
    [/nothing is sent when a scan merely finishes|nothing is sent.*scan/, "no automatic post-scan upload"],
    [/credential store/, "installed credential-store secret handling"],
    [/process memory/, "portable in-memory secret handling"],
    [/icmp|tcp/, "positive probe evidence"],
    [/arp cache|proxy-arp/, "macOS ARP finalization protection"],
  ];
  for (const [pattern, label] of claims) {
    if (!pattern.test(text)) throw new Error(\`the section does not cover \${label}\`);
  }

  const headings = await section.locator("h3").allInnerTexts();
  if (headings.length !== 3) throw new Error(\`expected 3 improvements, got \${headings.length}\`);
  return headings.map((h) => h.trim()).join(", ");
});

await step("the What changed link opens the local 1.8.5 page", async () => {`;

s = s.slice(0, start) + releaseBlock + s.slice(end + releaseEnd.length);
replaceRequired(
  'if (href !== "whats-new-1.8.4.html") {',
  'if (href !== "whats-new-1.8.5.html") {',
  "current What changed href",
);

const shotStart = 'await step("the new screenshots load at their stated size", async () => {';
const shotEnd = '\n});\n\nawait step("partial scans are described accurately", async () => {';
const shotStartIndex = s.indexOf(shotStart);
const shotEndIndex = s.indexOf(shotEnd, shotStartIndex);
if (shotStartIndex < 0 || shotEndIndex < 0) throw new Error("Could not locate stale screenshot assertion");

const shotBlock = `await step("the current product screenshots load at their stated size", async () => {
  await page.goto(BASE, { waitUntil: "networkidle" });
  const hero = page.locator('.hero img[src="assets/shots/inventory-dark.webp"]').first();
  if ((await hero.count()) === 0) throw new Error("the current inventory hero screenshot is missing");
  const info = await hero.evaluate((el) => ({
    complete: el.complete,
    natural: el.naturalWidth,
    w: el.getAttribute("width"),
    h: el.getAttribute("height"),
    alt: el.getAttribute("alt") ?? "",
  }));
  if (!info.complete || info.natural === 0) throw new Error("the inventory hero screenshot did not load");
  if (!info.w || !info.h) throw new Error("the inventory hero screenshot has no width/height attributes");
  if (info.alt.trim().length < 30) throw new Error("the inventory hero screenshot needs descriptive alt text");

  const tab = page.locator("#tab-partial");
  if ((await tab.count()) === 0) throw new Error("no partial-scan tab in the switcher");
  await tab.click();
  await page.waitForTimeout(250);
  const shown = await page.locator("#shot-image").getAttribute("src");
  if (!shown?.includes("history-partial-dark")) {
    throw new Error(\`the partial-scan tab shows \${shown}\`);
  }
  await page.locator("#tab-inventory").click();
  await page.waitForTimeout(150);
  return "hero screenshot plus the partial-scan switcher view";
});

await step("partial scans are described accurately", async () => {`;

s = s.slice(0, shotStartIndex) + shotBlock + s.slice(shotEndIndex + shotEnd.length);

replaceRequired(
  '    "whats-new-1.8.4.html",\n    "whats-new-1.8.3.html",',
  '    "whats-new-1.8.5.html",\n    "whats-new-1.8.4.html",\n    "whats-new-1.8.3.html",',
  "sitemap current release list",
);

const crossStart = 'await step("the home page and the What\\'s New page reach each other", async () => {';
let crossIndex = s.indexOf(crossStart);
if (crossIndex < 0) {
  crossIndex = s.indexOf('await step("the home page and the What\'s New page reach each other", async () => {');
}
if (crossIndex < 0) throw new Error("Could not find cross-link assertion");
const crossEnd = s.indexOf("\n});", crossIndex);
if (crossEnd < 0) throw new Error("Could not find cross-link assertion end");

const crossBlock = `await step("the home page and the current What's New page reach each other", async () => {
  const currentWhatsNew = "/whats-new-1.8.5.html";
  await page.goto(\`\${BASE}\${currentWhatsNew}\`, { waitUntil: "networkidle" });
  await page.locator('.hero a[href="./"]').first().click();
  await page.waitForLoadState("networkidle");
  if (!/See every device/.test(await page.locator("h1").innerText())) {
    throw new Error("the back link did not reach the home page");
  }
  await page.locator("#release-notes-link").click();
  await page.waitForLoadState("networkidle");
  const heading = await page.locator("h1").innerText();
  if (!/Observed inventory, straight into ArcAtlas/i.test(heading)) {
    throw new Error(\`the What changed link landed on: \${heading}\`);
  }
  return "home and 1.8.5 release page link both ways";
});`;

s = s.slice(0, crossIndex) + crossBlock + s.slice(crossEnd + 4);
writeFileSync(path, s);
console.log("Updated site verifier for ArcScan 1.8.5.");
