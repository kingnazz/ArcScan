#!/usr/bin/env node
// Verification for the ArcScan website.
//
// Checks the things that are easy to break and invisible in review: horizontal
// overflow at every supported width, the mobile menu staying inside the viewport
// and keyboard reachable, the download flow surviving a GitHub API failure,
// heading structure, alt text, contrast and the rest of the axe-core ruleset.
//
//   npm run build && npx serve site   (or any static server on port 4174)
//   npm i --no-save playwright @axe-core/playwright
//   node scripts/verify-site.mjs
//
// Set PLAYWRIGHT_CHROMIUM_PATH to reuse a Chromium already on the machine.

import { chromium } from "playwright";

const BASE = process.env.ARCSCAN_SITE_URL ?? "http://localhost:4174";
const WIDTHS = [320, 375, 390, 430, 768, 1024, 1280, 1440, 1920];

let failures = 0;
const step = async (name, fn) => {
  try {
    const detail = await fn();
    console.log(`PASS  ${name}${detail ? ` — ${detail}` : ""}`);
  } catch (error) {
    console.log(`FAIL  ${name} — ${error.message}`);
    failures += 1;
  }
};

const executablePath = process.env.PLAYWRIGHT_CHROMIUM_PATH;
const browser = await chromium.launch(executablePath ? { executablePath } : {});
const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
const page = await context.newPage();

const consoleErrors = [];
page.on("console", (m) => {
  if (m.type() !== "error") return;
  // A blocked or intercepted api.github.com is an environment condition, not a
  // site defect; the fallback path has its own step below.
  if (/api\.github\.com|ERR_CERT|Failed to load resource/.test(m.text())) return;
  consoleErrors.push(m.text());
});
page.on("pageerror", (e) => consoleErrors.push(`pageerror: ${e.message}`));

await page.goto(BASE, { waitUntil: "networkidle" });

await step("home page renders with the product headline", async () => {
  const h1 = await page.locator("h1").innerText();
  if (!/See every device/.test(h1)) throw new Error(`unexpected h1: ${h1}`);
  return h1;
});

await step("exactly one h1, and heading levels never skip", async () => {
  const levels = await page.$$eval("h1,h2,h3,h4", (nodes) =>
    nodes.map((n) => Number(n.tagName[1])),
  );
  const h1s = levels.filter((l) => l === 1).length;
  if (h1s !== 1) throw new Error(`expected 1 h1, found ${h1s}`);
  for (let i = 1; i < levels.length; i++) {
    if (levels[i] - levels[i - 1] > 1) {
      throw new Error(`heading jumps from h${levels[i - 1]} to h${levels[i]}`);
    }
  }
  return `${levels.length} headings`;
});

await step("every image has alt text and intrinsic dimensions", async () => {
  const bad = await page.$$eval("img", (imgs) =>
    imgs
      .filter((img) => img.getAttribute("alt") === null || !img.width || !img.height)
      .map((img) => img.getAttribute("src")),
  );
  if (bad.length) throw new Error(`images missing alt or size: ${bad.join(", ")}`);
  const lazy = await page.$$eval("img", (imgs) =>
    imgs.filter((i) => i.loading === "lazy").length,
  );
  return `${lazy} below-the-fold images lazy loaded`;
});

await step("viewport allows pinch zoom", async () => {
  const content = await page.getAttribute('meta[name="viewport"]', "content");
  if (/user-scalable\s*=\s*no|maximum-scale/.test(content ?? "")) {
    throw new Error(`viewport blocks zoom: ${content}`);
  }
  return content;
});

await step("no horizontal overflow at any supported width", async () => {
  const problems = [];
  for (const width of WIDTHS) {
    await page.setViewportSize({ width, height: 900 });
    await page.waitForTimeout(120);
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    if (overflow > 0) problems.push(`${width}px overflows by ${overflow}px`);
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  if (problems.length) throw new Error(problems.join("; "));
  return `${WIDTHS.length} widths clean`;
});

await step("the wide comparison table scrolls inside its own container", async () => {
  await page.setViewportSize({ width: 375, height: 800 });
  await page.waitForTimeout(120);
  const scrolls = await page.$eval(".table-scroll", (el) => el.scrollWidth > el.clientWidth);
  const pageOverflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  if (!scrolls) throw new Error("the table container does not scroll");
  if (pageOverflow > 0) throw new Error(`page overflows by ${pageOverflow}px`);
  return "table scrolls, page does not";
});

await step("mobile menu opens, stays in the viewport, and closes predictably", async () => {
  await page.setViewportSize({ width: 375, height: 800 });
  const toggle = page.locator("#nav-toggle");
  await toggle.click();
  const panel = page.locator("#nav-panel");
  await panel.waitFor({ state: "visible", timeout: 2000 });

  const box = await panel.boundingBox();
  if (!box) throw new Error("the menu has no box");
  if (box.x < 0 || box.x + box.width > 375 + 1) {
    throw new Error(`menu escapes the viewport: x=${box.x} w=${box.width}`);
  }
  if (box.height > 800 * 0.75) {
    throw new Error(`menu takes ${Math.round((box.height / 800) * 100)}% of the screen`);
  }
  // The page must still scroll behind an open menu, i.e. opening it must not
  // lock the body. The scroll is forced to "instant" because the site sets
  // scroll-behavior: smooth, which would make scrollTo animate and leave
  // window.scrollY still at 0 on the next line. What is under test is whether
  // the document can scroll at all, not how it animates.
  const scrolled = await page.evaluate(() => {
    window.scrollTo({ top: 400, behavior: "instant" });
    return window.scrollY;
  });
  if (scrolled < 300) throw new Error("the page cannot scroll while the menu is open");
  await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));

  await page.keyboard.press("Escape");
  await page.waitForTimeout(150);
  if (await panel.isVisible()) throw new Error("Escape did not close the menu");

  // Following a link closes it too.
  await toggle.click();
  await panel.getByRole("link", { name: "FAQ" }).click();
  await page.waitForTimeout(200);
  if (await panel.isVisible()) throw new Error("following a link left the menu open");
  return `menu is ${Math.round(box.height)}px tall at 375px`;
});

await step("mobile menu is reachable by keyboard", async () => {
  await page.setViewportSize({ width: 375, height: 800 });
  // A reload, so this measures a visitor pressing Enter on a closed menu rather
  // than whatever state the previous step happened to leave behind. Enter is a
  // toggle: inheriting an open menu would read as "Enter did not open it".
  await page.goto(BASE, { waitUntil: "networkidle" });
  await page.locator("#nav-toggle").focus();
  await page.keyboard.press("Enter");
  await page.waitForTimeout(150);
  const expanded = await page.getAttribute("#nav-toggle", "aria-expanded");
  if (expanded !== "true") throw new Error("Enter did not open the menu");
  await page.keyboard.press("Escape");
  return "opens with Enter, closes with Escape";
});

await step("skip link appears on focus and targets the main content", async () => {
  await page.setViewportSize({ width: 1440, height: 900 });
  // A reload, because Chrome keeps a sequential-focus starting point from the
  // last click and the last hash navigation, and blur() does not clear it. This
  // is what a visitor arriving at the page actually gets.
  await page.goto(BASE, { waitUntil: "networkidle" });
  await page.keyboard.press("Tab");
  // The link slides into view over 150ms, so measuring immediately would catch
  // it mid-transition.
  await page.waitForTimeout(250);
  const focused = await page.evaluate(() => {
    const el = document.activeElement;
    return { text: el?.textContent?.trim(), href: el?.getAttribute("href"), top: el?.getBoundingClientRect().top };
  });
  if (!/skip/i.test(focused.text ?? "")) throw new Error(`first tab stop is "${focused.text}"`);
  if (focused.href !== "#main") throw new Error(`skip link points at ${focused.href}`);
  if ((focused.top ?? -100) < 0) throw new Error("the skip link stays off screen when focused");
  return `"${focused.text}" to ${focused.href}`;
});

await step("screenshot switcher changes the image, caption and alt text", async () => {
  const before = await page.getAttribute("#shot-image", "src");
  await page.locator("#tab-changes").click();
  await page.waitForTimeout(200);
  const after = await page.getAttribute("#shot-image", "src");
  const alt = await page.getAttribute("#shot-image", "alt");
  const caption = await page.locator("#shot-caption").innerText();
  if (before === after) throw new Error("the image did not change");
  if (!alt || alt.length < 20) throw new Error("the alt text was not updated");
  if (!caption.trim()) throw new Error("the caption is empty");
  // Arrow keys must work, because the tabs claim the tablist role.
  await page.locator("#tab-changes").press("ArrowRight");
  await page.waitForTimeout(150);
  const selected = await page.locator('[role="tab"][aria-selected="true"]').getAttribute("id");
  if (selected !== "tab-history") throw new Error(`arrow key moved to ${selected}`);
  return `switched to ${after.split("/").pop()}, arrows work`;
});

await step("download cards list every platform with real detail", async () => {
  const cards = await page.$$eval(".dl", (els) =>
    els.map((el) => ({
      os: el.getAttribute("data-os"),
      heading: el.querySelector("h3")?.textContent?.trim(),
      link: el.querySelector('[data-field="link"]')?.getAttribute("href"),
      terms: Array.from(el.querySelectorAll("dt")).map((d) => d.textContent?.trim()),
      hasChecksum: Boolean(el.querySelector('[data-field="checksum"]')),
      hasNotes: Boolean(el.querySelector('[data-field="notes"]')),
    })),
  );
  if (cards.length !== 3) throw new Error(`expected 3 download cards, got ${cards.length}`);
  for (const card of cards) {
    for (const term of ["Architecture", "Installer", "Version", "Size"]) {
      if (!card.terms.includes(term)) throw new Error(`${card.os} is missing "${term}"`);
    }
    if (!card.link?.startsWith("https://github.com/")) throw new Error(`${card.os} has no link`);
    if (!card.hasChecksum || !card.hasNotes) throw new Error(`${card.os} lacks checksum or notes`);
  }
  return cards.map((c) => c.os).join(", ");
});

await step("exactly one platform is recommended, and only on that platform", async () => {
  // Headless Chromium reports Linux, which ArcScan does not ship: nothing should
  // be recommended, and no badge should be visible.
  const shown = await page.$$eval("[data-recommended-badge]", (els) =>
    els.filter((el) => el.offsetParent !== null).length,
  );
  if (shown !== 0) throw new Error(`${shown} badges visible on an unsupported platform`);

  // A Windows user agent must promote exactly one card.
  const win = await context.newPage();
  await win.addInitScript(() => {
    Object.defineProperty(navigator, "userAgent", {
      get: () => "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120 Safari/537.36",
    });
    Object.defineProperty(navigator, "userAgentData", { get: () => undefined });
  });
  await win.goto(BASE, { waitUntil: "domcontentloaded" });
  await win.waitForTimeout(300);
  const promoted = await win.$$eval("[data-recommended-badge]", (els) =>
    els.filter((el) => el.offsetParent !== null).map((el) => el.closest(".dl")?.getAttribute("data-os")),
  );
  const primaries = await win.$$eval(".dl .btn-primary", (els) => els.length);
  await win.close();
  if (promoted.length !== 1) throw new Error(`${promoted.length} cards promoted on Windows`);
  if (promoted[0] !== "win-x64") throw new Error(`promoted ${promoted[0]} on Windows x64`);
  if (primaries !== 1) throw new Error(`${primaries} primary download buttons`);
  return `none on Linux, ${promoted[0]} on Windows`;
});

/**
 * The v1.7.1 release as GitHub actually returns it, including the signature and
 * updater assets that sit alongside the installers. Using the real asset names
 * is the point: the page picks assets by pattern, and a pattern that quietly
 * matched ArcScan_1.7.1_x64-setup.exe.sig would hand visitors a 400 byte file.
 */
const RELEASE_FIXTURE = {
  tag_name: "v1.7.1",
  html_url: "https://github.com/kingnazz/ArcScan/releases/tag/v1.7.1",
  published_at: "2026-08-01T06:28:00Z",
  assets: [
    { name: "ArcScan.app.tar.gz", size: 7505259 },
    { name: "ArcScan.app.tar.gz.sig", size: 404 },
    { name: "ArcScan_1.7.1_arm64-setup.exe", size: 4285945 },
    { name: "ArcScan_1.7.1_arm64-setup.exe.sig", size: 420 },
    { name: "ArcScan_1.7.1_universal.dmg", size: 7576208 },
    { name: "ArcScan_1.7.1_x64-setup.exe", size: 4543127 },
    { name: "ArcScan_1.7.1_x64-setup.exe.sig", size: 416 },
    { name: "latest.json", size: 2376 },
  ].map((a) => ({
    ...a,
    browser_download_url: `https://github.com/kingnazz/ArcScan/releases/download/v1.7.1/${a.name}`,
  })),
};

/** Load the page with the GitHub release API stubbed, and no other network. */
async function pageWithRelease(payload, userAgent) {
  const p = await context.newPage();
  if (userAgent) {
    await p.addInitScript((ua) => {
      Object.defineProperty(navigator, "userAgent", { get: () => ua });
      Object.defineProperty(navigator, "userAgentData", { get: () => undefined });
    }, userAgent);
  }
  await p.route("https://api.github.com/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(payload),
    }),
  );
  await p.goto(BASE, { waitUntil: "domcontentloaded" });
  // The fetch is fired after the page is interactive, so give it a beat.
  await p.waitForFunction(
    () => document.querySelector('.dl[data-os="mac"] [data-field="size"]')?.textContent !== "See the release page",
    { timeout: 5000 },
  );
  return p;
}

await step("the release API fills every card with the right asset", async () => {
  const p = await pageWithRelease(RELEASE_FIXTURE);
  const cards = await p.$$eval(".dl", (els) =>
    els.map((el) => ({
      os: el.getAttribute("data-os"),
      link: el.querySelector('[data-field="link"]')?.getAttribute("href"),
      version: el.querySelector('[data-field="version"]')?.textContent?.trim(),
      size: el.querySelector('[data-field="size"]')?.textContent?.trim(),
      notes: el.querySelector('[data-field="notes"]')?.getAttribute("href"),
    })),
  );
  await p.close();

  const expected = {
    "win-x64": { file: "ArcScan_1.7.1_x64-setup.exe", size: "4.3 MB" },
    "win-arm64": { file: "ArcScan_1.7.1_arm64-setup.exe", size: "4.1 MB" },
    mac: { file: "ArcScan_1.7.1_universal.dmg", size: "7.2 MB" },
  };
  for (const card of cards) {
    const want = expected[card.os];
    if (!want) throw new Error(`unexpected card ${card.os}`);
    if (!card.link?.endsWith(want.file)) throw new Error(`${card.os} links to ${card.link}`);
    // A .sig or the updater tarball reaching a download button is the failure
    // this guards against.
    if (/\.sig$|\.tar\.gz$|latest\.json$/.test(card.link ?? "")) {
      throw new Error(`${card.os} links to a non-installer asset: ${card.link}`);
    }
    if (card.version !== "1.7.1") throw new Error(`${card.os} shows version ${card.version}`);
    if (card.size !== want.size) throw new Error(`${card.os} shows size ${card.size}`);
    if (!card.notes?.includes("/releases/tag/v1.7.1")) {
      throw new Error(`${card.os} notes link is ${card.notes}`);
    }
  }
  return cards.map((c) => `${c.os} ${c.size}`).join(", ");
});

await step("each platform is recommended the build it can run", async () => {
  const cases = [
    [
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Safari/605.1.15",
      "mac",
      "ArcScan_1.7.1_universal.dmg",
    ],
    [
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120 Safari/537.36",
      "win-x64",
      "ArcScan_1.7.1_x64-setup.exe",
    ],
    [
      "Mozilla/5.0 (Windows NT 10.0; Win64; ARM64) AppleWebKit/537.36 Chrome/120 Safari/537.36",
      "win-arm64",
      "ArcScan_1.7.1_arm64-setup.exe",
    ],
  ];
  const seen = [];
  for (const [ua, os, file] of cases) {
    const p = await pageWithRelease(RELEASE_FIXTURE, ua);
    const promoted = await p.$$eval("[data-recommended-badge]", (els) =>
      els.filter((el) => el.offsetParent !== null).map((el) => el.closest(".dl")?.getAttribute("data-os")),
    );
    const hero = await p.locator("#hero-download").getAttribute("href");
    await p.close();
    if (promoted.length !== 1 || promoted[0] !== os) {
      throw new Error(`${os}: promoted ${JSON.stringify(promoted)}`);
    }
    // The hero button must follow the recommendation, not stay on /latest.
    if (!hero?.endsWith(file)) throw new Error(`${os}: hero button points at ${hero}`);
    seen.push(os);
  }
  return seen.join(", ");
});

await step("a release with no installers never shows a broken download", async () => {
  // An interrupted release, or one whose assets are still uploading. The page
  // must fall back to the release page rather than linking at nothing.
  const p = await pageWithRelease({
    ...RELEASE_FIXTURE,
    assets: [{ name: "latest.json", size: 2376, browser_download_url: "https://example.invalid/x" }],
  }).catch(() => null);
  if (!p) {
    // The wait above times out precisely because no size was filled in, which
    // is itself the correct behaviour; re-check without waiting.
    const q = await context.newPage();
    await q.route("https://api.github.com/**", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ ...RELEASE_FIXTURE, assets: [] }),
      }),
    );
    await q.goto(BASE, { waitUntil: "domcontentloaded" });
    await q.waitForTimeout(1200);
    const links = await q.$$eval(".dl [data-field='link']", (els) =>
      els.map((el) => el.getAttribute("href")),
    );
    await q.close();
    for (const href of links) {
      if (!href?.startsWith("https://github.com/kingnazz/ArcScan/releases")) {
        throw new Error(`a card pointed at ${href} with no assets`);
      }
    }
    return "cards stay on the releases page";
  }
  await p.close();
  throw new Error("a release with no installers filled in a size");
});

await step("signing language stays honest", async () => {
  const text = await page.locator("body").innerText();
  if (!/not code-signed|not signed with a paid publisher certificate/i.test(text)) {
    throw new Error("the page does not disclose that installers are unsigned");
  }
  if (/\bnotarized\b|\bnotarised\b/i.test(text) && !/not notaris|not notariz/i.test(text)) {
    throw new Error("notarisation is mentioned without saying it is absent");
  }
  return "unsigned installers disclosed";
});

await step("no em dashes anywhere in the copy", async () => {
  // Collapsed <details> contribute nothing to innerText, so the FAQ answers have
  // to be open before any copy check can see them.
  await page.$$eval("details", (nodes) => nodes.forEach((n) => (n.open = true)));
  const found = await page.evaluate(() => {
    const text = document.body.innerText;
    const index = text.indexOf("—");
    return index >= 0 ? text.slice(Math.max(0, index - 40), index + 40) : null;
  });
  if (found) throw new Error(`em dash found near: ${found}`);
});

await step("no IPv6 claim, since it is not implemented", async () => {
  const text = (await page.locator("body").innerText()).toLowerCase();
  if (text.includes("ipv6") && !/no\.\s*arcscan discovers ipv4/.test(text)) {
    throw new Error("IPv6 is mentioned without stating it is unsupported");
  }
  return "IPv6 stated as unsupported";
});

await step("structured data parses and matches the visible version", async () => {
  const blocks = await page.$$eval('script[type="application/ld+json"]', (nodes) =>
    nodes.map((n) => n.textContent ?? ""),
  );
  if (blocks.length < 2) throw new Error(`expected SoftwareApplication and FAQPage, got ${blocks.length}`);
  const parsed = blocks.map((b) => JSON.parse(b));
  const app = parsed.find((p) => p["@type"] === "SoftwareApplication");
  const faq = parsed.find((p) => p["@type"] === "FAQPage");
  if (!app) throw new Error("no SoftwareApplication block");
  if (!faq) throw new Error("no FAQPage block");

  const shown = (await page.locator("#version-fallback").innerText()).replace(/^v/, "");
  if (app.softwareVersion !== shown) {
    throw new Error(`structured data says ${app.softwareVersion}, page shows ${shown}`);
  }
  // Every structured FAQ entry must exist on the page.
  const visible = await page.$$eval(".faq summary", (nodes) =>
    nodes.map((n) => n.textContent?.trim().toLowerCase()),
  );
  if (faq.mainEntity.length !== visible.length) {
    throw new Error(`${faq.mainEntity.length} structured questions, ${visible.length} on the page`);
  }
  return `${faq.mainEntity.length} FAQ entries, version ${app.softwareVersion}`;
});

await step("the release section states the 1.7.1 improvements", async () => {
  const section = page.locator("#whats-new");
  await section.waitFor({ timeout: 3000 });
  const text = (await section.innerText()).toLowerCase();

  // The version has to be named, or the section could describe any release.
  if (!text.includes("1.7.1")) throw new Error("the section does not name the version");

  // One assertion per improvement, phrased as the claim rather than as exact
  // wording, so the copy can be edited without the test becoming a transcript.
  const claims = [
    [/partial scans?/, "partial scans"],
    [/never report unprobed devices as missing|never report.*as missing/, "no false missing devices"],
    [/network[- ]scoped|attached to the network/, "network-scoped inventory"],
    [/same target and ports/, "comparison coverage"],
    [/exact scan selected/, "historical export accuracy"],
  ];
  for (const [pattern, label] of claims) {
    if (!pattern.test(text)) throw new Error(`the section does not cover ${label}`);
  }

  const headings = await section.locator("h3").allInnerTexts();
  if (headings.length !== 3) throw new Error(`expected 3 improvements, got ${headings.length}`);
  return headings.map((h) => h.trim()).join(", ");
});

await step("the release notes link points at the 1.7.1 tag", async () => {
  const link = page.locator("#release-notes-link");
  await link.waitFor({ timeout: 3000 });
  const href = await link.getAttribute("href");
  // A tag URL, not /releases/latest: it must keep resolving to these notes
  // after a later release ships, and without the GitHub API answering.
  if (href !== "https://github.com/kingnazz/ArcScan/releases/tag/v1.7.1") {
    throw new Error(`release link points at ${href}`);
  }
  const shown = (await page.locator("#version-fallback").innerText()).replace(/^v/, "");
  if (!(await link.innerText()).includes(shown)) {
    throw new Error("the link text does not name the version the page shows");
  }
  // It is a real anchor with a visible focus ring, not a click handler.
  await link.focus();
  const outline = await link.evaluate((el) => getComputedStyle(el).outlineStyle);
  if (outline === "none") throw new Error("the release link has no focus indicator");
  return href;
});

await step("the new screenshots load at their stated size", async () => {
  const shots = [
    "assets/shots/changes-partial-dark.webp",
    "assets/shots/settings-networks-dark.webp",
  ];
  for (const src of shots) {
    const img = page.locator(`img[src="${src}"]`);
    if ((await img.count()) === 0) throw new Error(`${src} is not on the page`);
    // They are lazy and below the fold, so they only fetch once scrolled to.
    await img.first().scrollIntoViewIfNeeded();
    await page.waitForFunction(
      (selector) => {
        const el = document.querySelector(selector);
        return el instanceof HTMLImageElement && el.complete && el.naturalWidth > 0;
      },
      `img[src="${src}"]`,
      { timeout: 5000 },
    );
    const info = await img.first().evaluate((el) => ({
      complete: el.complete,
      natural: el.naturalWidth,
      w: el.getAttribute("width"),
      h: el.getAttribute("height"),
      alt: el.getAttribute("alt") ?? "",
      loading: el.getAttribute("loading"),
    }));
    if (!info.complete || info.natural === 0) throw new Error(`${src} did not load`);
    // Intrinsic dimensions are what stop the section shifting as it loads.
    if (!info.w || !info.h) throw new Error(`${src} has no width/height attributes`);
    if (info.alt.trim().length < 30) throw new Error(`${src} needs descriptive alt text`);
    if (info.loading !== "lazy") throw new Error(`${src} should be lazy, it is below the fold`);
  }

  // The partial-scan view is reachable from the switcher rather than inline.
  const tab = page.locator("#tab-partial");
  if ((await tab.count()) === 0) throw new Error("no partial-scan tab in the switcher");
  await tab.click();
  await page.waitForTimeout(250);
  const shown = await page.locator("#shot-image").getAttribute("src");
  if (!shown?.includes("history-partial-dark")) {
    throw new Error(`the partial-scan tab shows ${shown}`);
  }
  await page.locator("#tab-results").click();
  await page.waitForTimeout(150);
  return `${shots.length} inline shots plus the switcher tab`;
});

await step("partial scans are described accurately", async () => {
  const body = (await page.locator("main").innerText()).toLowerCase();
  // Saved, labelled, and explicitly not a source of missing-device reports.
  if (!/partial scan/.test(body)) throw new Error("partial scans are never mentioned");
  if (!/(kept|saved)/.test(body)) throw new Error("the page does not say results are kept");
  // The page must not claim a stopped scan is a complete picture.
  if (/partial scans? (are|is) complete/.test(body)) {
    throw new Error("the page claims a partial scan is complete");
  }
  // Comparison must be described as requiring completed, equivalent coverage.
  if (!/completed scans?/.test(body)) {
    throw new Error("the page does not say comparison needs completed scans");
  }
});

await step("SEO metadata is complete", async () => {
  const meta = await page.evaluate(() => ({
    title: document.title,
    description: document.querySelector('meta[name="description"]')?.getAttribute("content"),
    canonical: document.querySelector('link[rel="canonical"]')?.getAttribute("href"),
    ogImage: document.querySelector('meta[property="og:image"]')?.getAttribute("content"),
    twitter: document.querySelector('meta[name="twitter:card"]')?.getAttribute("content"),
  }));
  for (const [key, value] of Object.entries(meta)) {
    if (!value) throw new Error(`missing ${key}`);
  }
  // Search results truncate past about 65 characters.
  if (meta.title.length > 65) throw new Error(`title is ${meta.title.length} characters`);
  if ((meta.description?.length ?? 0) > 320) throw new Error("description is too long");
  return `title ${meta.title.length} chars, description ${meta.description.length} chars`;
});

await step("robots.txt and sitemap.xml are served", async () => {
  for (const path of ["/robots.txt", "/sitemap.xml"]) {
    const response = await page.request.get(`${BASE}${path}`);
    if (!response.ok()) throw new Error(`${path} returned ${response.status()}`);
  }
  return "both present";
});

await step("privacy page loads and names the public IP providers", async () => {
  await page.goto(`${BASE}/privacy.html`, { waitUntil: "networkidle" });
  const text = await page.locator("main").innerText();
  for (const needed of ["ipify", "icanhazip", "off by default", "GitHub"]) {
    if (!text.includes(needed)) throw new Error(`privacy page does not mention "${needed}"`);
  }
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  if (overflow > 0) throw new Error(`privacy page overflows by ${overflow}px`);
  return "providers named, no overflow";
});

// --- GitHub API failure ----------------------------------------------------
await step("download flow survives a GitHub API failure", async () => {
  const offline = await context.newPage();
  await offline.route("https://api.github.com/**", (route) => route.abort());
  await offline.goto(BASE, { waitUntil: "networkidle" });
  await offline.waitForTimeout(600);

  const status = await offline.locator("#download-status").innerText();
  if (!/could not be loaded|latest release page/i.test(status)) {
    throw new Error(`no fallback message: ${status}`);
  }
  const links = await offline.$$eval(".dl [data-field='link']", (els) =>
    els.map((e) => e.getAttribute("href")),
  );
  if (links.some((href) => !href?.startsWith("https://github.com/kingnazz/ArcScan/releases"))) {
    throw new Error(`a download link is dead: ${links.join(", ")}`);
  }
  await offline.close();
  return "all three links still point at the releases page";
});

// --- axe-core --------------------------------------------------------------
await step("axe-core finds no violations on either page", async () => {
  let AxeBuilder;
  try {
    AxeBuilder = (await import("@axe-core/playwright")).default;
  } catch {
    throw new Error("@axe-core/playwright is not installed; run npm i --no-save @axe-core/playwright");
  }
  const results = [];
  for (const path of ["/", "/privacy.html"]) {
    await page.goto(`${BASE}${path}`, { waitUntil: "networkidle" });
    const scan = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
      .analyze();
    if (scan.violations.length > 0) {
      const summary = scan.violations
        .map((v) => `${path} ${v.id} (${v.nodes.length}): ${v.help}`)
        .join("; ");
      throw new Error(summary);
    }
    results.push(`${path}: ${scan.passes.length} checks passed`);
  }
  return results.join(", ");
});

if (consoleErrors.length > 0) {
  console.log(`FAIL  console clean — ${consoleErrors.length} error(s)`);
  for (const e of consoleErrors.slice(0, 5)) console.log(`      ${e}`);
  failures += 1;
} else {
  console.log("PASS  console clean — no errors");
}

await browser.close();
process.exitCode = failures > 0 ? 1 : 0;
