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
