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
  const ids = await page.$$eval('[role="tab"]', (els) => els.map((e) => e.id));
  const next = ids[ids.indexOf("tab-changes") + 1];
  await page.locator("#tab-changes").press("ArrowRight");
  await page.waitForTimeout(150);
  const selected = await page.locator('[role="tab"][aria-selected="true"]').getAttribute("id");
  if (selected !== next) throw new Error(`arrow key moved to ${selected}, expected ${next}`);
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
      options: Array.from(el.querySelectorAll(".dl-option h4")).map((h) => h.textContent?.trim()),
    })),
  );
  if (cards.length !== 3) throw new Error(`expected 3 download cards, got ${cards.length}`);
  for (const card of cards) {
    for (const term of ["Architecture", "Version", "Size"]) {
      if (!card.terms.includes(term)) throw new Error(`${card.os} is missing "${term}"`);
    }
    if (!card.link?.startsWith("https://github.com/")) throw new Error(`${card.os} has no link`);
    if (!card.hasChecksum || !card.hasNotes) throw new Error(`${card.os} lacks checksum or notes`);
    if (!card.options.includes("Installer")) throw new Error(`${card.os} has no Installer option`);
  }
  // Both Windows cards offer a portable ZIP; macOS must not, because there is
  // no portable macOS build and a card implying one would be a false claim on
  // the download page itself.
  const portable = cards.filter((c) => c.options.includes("Portable ZIP")).map((c) => c.os).sort();
  if (JSON.stringify(portable) !== JSON.stringify(["win-arm64", "win-x64"])) {
    throw new Error(`portable options on ${JSON.stringify(portable)}`);
  }
  return cards.map((c) => `${c.os} (${c.options.join(" + ")})`).join(", ");
});

await step("exactly one platform is recommended, and only on that platform", async () => {
  // Pin an unsupported user agent instead of inheriting the host running this
  // suite: local macOS Chromium and CI Linux report different real platforms.
  const unsupported = await context.newPage();
  await unsupported.addInitScript(() => {
    Object.defineProperty(navigator, "userAgent", {
      get: () => "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120 Safari/537.36",
    });
    Object.defineProperty(navigator, "userAgentData", { get: () => undefined });
  });
  await unsupported.goto(BASE, { waitUntil: "domcontentloaded" });
  const shown = await unsupported.$$eval("[data-recommended-badge]", (els) =>
    els.filter((el) => el.offsetParent !== null).length,
  );
  await unsupported.close();
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
 * A release as GitHub actually returns one, including the signature and
 * updater assets that sit alongside the installers. Using the real asset names
 * is the point: the page picks assets by pattern, and a pattern that quietly
 * matched ArcScan_1.8.4_x64-setup.exe.sig would hand visitors a 400 byte file.
 */
const RELEASE_FIXTURE = {
  tag_name: "v1.8.4",
  html_url: "https://github.com/kingnazz/ArcScan/releases/tag/v1.8.4",
  published_at: "2026-08-01T06:28:00Z",
  assets: [
    // Stale assets are deliberately first. Selection is version-specific, so
    // an old filename attached to the current release cannot win by API order.
    { name: "ArcScan_1.8.3_x64-setup.exe", size: 4540000 },
    { name: "ArcScan_1.8.3_arm64-setup.exe", size: 4280000 },
    { name: "ArcScan_1.8.3_universal.dmg", size: 7500000 },
    { name: "ArcScan_1.8.3_windows-x64-portable.zip", size: 4100000 },
    { name: "ArcScan_1.8.3_windows-arm64-portable.zip", size: 3900000 },
    { name: "ArcScan.app.tar.gz", size: 7505259 },
    { name: "ArcScan.app.tar.gz.sig", size: 404 },
    { name: "ArcScan_1.8.4_arm64-setup.exe", size: 4285945 },
    { name: "ArcScan_1.8.4_arm64-setup.exe.sig", size: 420 },
    { name: "ArcScan_1.8.4_universal.dmg", size: 7576208 },
    { name: "ArcScan_1.8.4_x64-setup.exe", size: 4543127 },
    { name: "ArcScan_1.8.4_x64-setup.exe.sig", size: 416 },
    // The portable ZIPs, unsigned as updater artifacts by design. Their names
    // are close enough to the installers' that a loose pattern would confuse
    // them, which is the point of having them in the fixture.
    { name: "ArcScan_1.8.4_windows-x64-portable.zip", size: 4194304 },
    { name: "ArcScan_1.8.4_windows-arm64-portable.zip", size: 3984588 },
    { name: "latest.json", size: 2376 },
  ].map((a) => ({
    ...a,
    browser_download_url: `https://github.com/kingnazz/ArcScan/releases/download/v1.8.4/${a.name}`,
  })),
};

/** Load the page with the GitHub release API stubbed, and no other network. */
async function pageWithRelease(payload, userAgent, path = "/") {
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
  await p.goto(`${BASE}${path}`, { waitUntil: "domcontentloaded" });
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
      portableLink: el.querySelector('[data-field="portable-link"]')?.getAttribute("href"),
      portableSize: el.querySelector('[data-field="portable-size"]')?.textContent?.trim(),
    })),
  );
  await p.close();

  const expected = {
    "win-x64": {
      file: "ArcScan_1.8.4_x64-setup.exe",
      size: "4.3 MB",
      portable: "ArcScan_1.8.4_windows-x64-portable.zip",
      portableSize: "4.0 MB",
    },
    "win-arm64": {
      file: "ArcScan_1.8.4_arm64-setup.exe",
      size: "4.1 MB",
      portable: "ArcScan_1.8.4_windows-arm64-portable.zip",
      portableSize: "3.8 MB",
    },
    mac: { file: "ArcScan_1.8.4_universal.dmg", size: "7.2 MB", portable: null },
  };
  for (const card of cards) {
    const want = expected[card.os];
    if (!want) throw new Error(`unexpected card ${card.os}`);
    if (!card.link?.endsWith(want.file)) throw new Error(`${card.os} links to ${card.link}`);
    // A .sig, the updater tarball, or a portable ZIP reaching the *installer*
    // button is the failure this guards against.
    if (/\.sig$|\.tar\.gz$|latest\.json$|portable/i.test(card.link ?? "")) {
      throw new Error(`${card.os} installer links to ${card.link}`);
    }
    if (card.version !== "1.8.4") throw new Error(`${card.os} shows version ${card.version}`);
    if (card.size !== want.size) throw new Error(`${card.os} shows size ${card.size}`);
    if (!card.notes?.includes("/releases/tag/v1.8.4")) {
      throw new Error(`${card.os} notes link is ${card.notes}`);
    }

    if (want.portable === null) {
      if (card.portableLink) {
        throw new Error(`${card.os} offers a portable download, which does not exist`);
      }
      continue;
    }
    if (!card.portableLink?.endsWith(want.portable)) {
      throw new Error(`${card.os} portable links to ${card.portableLink}`);
    }
    // And the architectures must not cross. This is the mistake with the
    // quietest failure: the download works, the app launches, and it is the
    // wrong build.
    const other = card.os === "win-x64" ? /arm64/i : /(^|[^a-z])x64/i;
    if (other.test(card.portableLink.split("/").pop() ?? "")) {
      throw new Error(`${card.os} portable links to the other architecture: ${card.portableLink}`);
    }
    if (card.portableSize !== want.portableSize) {
      throw new Error(`${card.os} portable shows size ${card.portableSize}`);
    }
  }
  return cards.map((c) => `${c.os} ${c.size}${c.portableSize ? ` + ${c.portableSize}` : ""}`).join(", ");
});

await step("release asset selection rejects ambiguous same-version matches", async () => {
  const ambiguous = {
    ...RELEASE_FIXTURE,
    assets: [
      ...RELEASE_FIXTURE.assets,
      {
        name: "ArcScan_1.8.4_backup_x64-setup.exe",
        size: 4543128,
        browser_download_url:
          "https://github.com/kingnazz/ArcScan/releases/download/v1.8.4/ArcScan_1.8.4_backup_x64-setup.exe",
      },
    ],
  };
  const p = await pageWithRelease(ambiguous);
  const card = await p.$eval('.dl[data-os="win-x64"]', (el) => ({
    href: el.querySelector('[data-field="link"]')?.getAttribute("href"),
    size: el.querySelector('[data-field="size"]')?.textContent?.trim(),
  }));
  await p.close();
  if (card.href !== "https://github.com/kingnazz/ArcScan/releases/latest") {
    throw new Error(`ambiguous x64 installer resolved to ${card.href}`);
  }
  if (card.size !== "See the release page") {
    throw new Error(`ambiguous x64 installer displayed size ${card.size}`);
  }
  return "ambiguous card keeps its safe release-page fallback";
});

await step("each platform is recommended the build it can run", async () => {
  const cases = [
    [
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Safari/605.1.15",
      "mac",
      "ArcScan_1.8.4_universal.dmg",
    ],
    [
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120 Safari/537.36",
      "win-x64",
      "ArcScan_1.8.4_x64-setup.exe",
    ],
    [
      "Mozilla/5.0 (Windows NT 10.0; Win64; ARM64) AppleWebKit/537.36 Chrome/120 Safari/537.36",
      "win-arm64",
      "ArcScan_1.8.4_arm64-setup.exe",
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
    const links = await q.$$eval(
      ".dl [data-field='link'], .dl [data-field='portable-link']",
      (els) => els.map((el) => el.getAttribute("href")),
    );
    const sizes = await q.$$eval(
      ".dl [data-field='size'], .dl [data-field='portable-size']",
      (els) => els.map((el) => el.textContent?.trim()),
    );
    await q.close();
    for (const href of links) {
      if (!href?.startsWith("https://github.com/kingnazz/ArcScan/releases")) {
        throw new Error(`a card pointed at ${href} with no assets`);
      }
    }
    for (const size of sizes) {
      if (size !== "See the release page") throw new Error(`a size read "${size}" with no assets`);
    }
    return `${links.length} controls stay on the releases page`;
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

await step("the release section states the 1.8.4 improvements", async () => {
  const section = page.locator("#whats-new");
  await section.waitFor({ timeout: 3000 });
  const text = (await section.innerText()).toLowerCase();

  // The version has to be named, or the section could describe any release.
  if (!text.includes("1.8.4")) throw new Error("the section does not name the version");

  // One assertion per claim, phrased as the claim rather than as exact wording,
  // so the copy can be edited without the test becoming a transcript.
  const claims = [
    [/portable/, "the portable edition"],
    [/windows/, "the platform it is for"],
    [/x64/, "x64"],
    [/arm64/, "ARM64"],
    [/disposable/, "the disposable field-tool model"],
    [/temporary session/, "temporary per-launch state"],
    [/sqlite|database/, "the in-session SQLite database"],
    [/webview/, "the isolated WebView profile"],
    [/next portable launch starts fresh|start empty next time/, "the fresh next launch"],
    [/csv, json and xml/, "the intentional export formats"],
    [/no installer|without installing|unzip/, "no installation being needed"],
    [/independent|fully separate/, "independence from the Installed copy"],
    [/two portable processes|separate sessions/, "concurrent Portable processes"],
    [/validated arcscan ownership/, "ownership-gated cleanup"],
    [/never falls back/, "the absence of a silent fallback"],
    [/unchanged/, "what did not change"],
  ];
  for (const [pattern, label] of claims) {
    if (!pattern.test(text)) throw new Error(`the section does not cover ${label}`);
  }

  // Claims from the superseded persistent-folder architecture must never return.
  for (const overclaim of [
    [/portable.{0,40}macos|macos.{0,40}portable/, "portable macOS"],
    [/zero dependencies|no dependencies/, "zero dependencies"],
    [/updates itself|self-updat|automatic.{0,20}portable update/, "portable self-update"],
    [/arcscandata/, "a persistent ArcScanData folder"],
    [/\b(?:move|copy) the whole folder\b/, "folder-carried persistence"],
    [/same-folder lock/, "the obsolete same-folder lock"],
  ]) {
    if (overclaim[0].test(text)) throw new Error(`the section claims ${overclaim[1]}`);
  }

  const headings = await section.locator("h3").allInnerTexts();
  if (headings.length !== 3) throw new Error(`expected 3 improvements, got ${headings.length}`);
  return headings.map((h) => h.trim()).join(", ");
});

await step("the What changed link opens the local 1.8.4 page", async () => {
  const link = page.locator("#release-notes-link");
  await link.waitFor({ timeout: 3000 });
  const href = await link.getAttribute("href");
  // A first-party page, not GitHub: a visitor asking what changed should get
  // something written for them before they get a commit list.
  if (href !== "whats-new-1.8.4.html") {
    throw new Error(`the What changed link points at ${href}`);
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
  const shots = ["assets/shots/settings-portable-dark.webp"];
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
  await page.locator("#tab-inventory").click();
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

await step("robots.txt and sitemap.xml are served, and the sitemap is current", async () => {
  for (const path of ["/robots.txt", "/sitemap.xml"]) {
    const response = await page.request.get(`${BASE}${path}`);
    if (!response.ok()) throw new Error(`${path} returned ${response.status()}`);
  }
  // A What's New page nothing links to from a sitemap is a page search engines
  // find late, and the 1.8.2 entry was missed once already.
  const sitemap = await (await page.request.get(`${BASE}/sitemap.xml`)).text();
  for (const page_ of [
    "whats-new-1.8.4.html",
    "whats-new-1.8.3.html",
    "whats-new-1.8.2.html",
    "whats-new-1.8.1.html",
    "whats-new-1.8.0.html",
    "privacy.html",
  ]) {
    if (!sitemap.includes(page_)) throw new Error(`the sitemap does not list ${page_}`);
  }
  return "both present, sitemap lists every page";
});

await step("privacy page loads and names the public IP providers", async () => {
  await page.goto(`${BASE}/privacy.html`, { waitUntil: "networkidle" });
  const text = await page.locator("main").innerText();
  for (const needed of ["ipify", "icanhazip", "GitHub"]) {
    if (!text.includes(needed)) throw new Error(`privacy page does not mention "${needed}"`);
  }
  // The lookup being offered by default is not the same as it running by
  // default, and the page has to be unambiguous about which one it means.
  for (const [pattern, label] of [
    [/never runs on its own/i, "that the lookup never runs on its own"],
    [/press\s+<?\/?strong>?\s*check|press Check/i, "that a press is what starts it"],
    [/never stored as inventory data|never written to the scan database/i,
      "that the answer is not stored"],
    [/switched off|switch(ed)? it off/i, "that it can be switched off"],
  ]) {
    if (!pattern.test(text)) throw new Error(`privacy page does not state ${label}`);
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


// ---------------------------------------------------------------------------
// The first-party What's New page for 1.8.4
// ---------------------------------------------------------------------------

const WHATS_NEW = "/whats-new-1.8.4.html";

await step("the What's New page loads with the right title and metadata", async () => {
  const response = await page.goto(`${BASE}${WHATS_NEW}`, { waitUntil: "networkidle" });
  if (!response || !response.ok()) {
    throw new Error(`the page returned ${response ? response.status() : "nothing"}`);
  }

  const title = await page.title();
  if (!/what's new in arcscan 1\.8/i.test(title)) throw new Error(`unexpected title: ${title}`);
  // Distinct from the home page's, or the two compete for the same query.
  if (/free network scanner and inventory/i.test(title)) {
    throw new Error("the page reuses the home page title");
  }

  const meta = async (selector) => page.getAttribute(selector, "content");
  const description = await meta('meta[name="description"]');
  if (!description || description.length < 80 || description.length > 320) {
    throw new Error(`meta description is ${description ? description.length : 0} characters`);
  }
  const canonical = await page.getAttribute('link[rel="canonical"]', "href");
  if (canonical !== `https://kingnazz.github.io/ArcScan${WHATS_NEW}`) {
    throw new Error(`canonical is ${canonical}`);
  }
  for (const selector of [
    'meta[property="og:title"]',
    'meta[property="og:description"]',
    'meta[property="og:image"]',
    'meta[property="og:url"]',
    'meta[name="twitter:card"]',
    'meta[name="twitter:title"]',
    'meta[name="twitter:image"]',
  ]) {
    const value = await meta(selector);
    if (!value) throw new Error(`${selector} is missing or empty`);
  }
  // The social image has to be an image that exists, not a guess.
  const ogImage = await meta('meta[property="og:image"]');
  const reachable = await page.evaluate(
    (src) =>
      new Promise((resolve) => {
        const probe = new Image();
        probe.onload = () => resolve(true);
        probe.onerror = () => resolve(false);
        probe.src = src.replace("https://kingnazz.github.io/ArcScan/", "");
      }),
    ogImage,
  );
  if (!reachable) throw new Error(`og:image does not resolve: ${ogImage}`);
  return `${title.length} char title, ${description.length} char description`;
});

await step("structured data names version 1.8.4 and parses", async () => {
  const blocks = await page.$$eval('script[type="application/ld+json"]', (nodes) =>
    nodes.map((n) => n.textContent),
  );
  if (blocks.length === 0) throw new Error("no structured data on the page");
  const parsed = blocks.map((b) => JSON.parse(b));
  const article = parsed.find((b) => b["@type"] === "Article");
  if (!article) throw new Error("no Article block");
  if (article.about?.softwareVersion !== "1.8.4") {
    throw new Error(`structured data says version ${article.about?.softwareVersion}`);
  }
  // No datePublished assertion: a page written before the release is published
  // must not carry an invented date, and the release date is not knowable here.
  return `Article, softwareVersion ${article.about.softwareVersion}`;
});

await step("the page states the version visibly", async () => {
  const text = await page.locator("main").innerText();
  if (!/1\.8\.4/.test(text)) throw new Error("the version is not visible on the page");
  if (!/free and open source/i.test(text)) throw new Error("the free and open source label is missing");
  return "version 1.8.4";
});

await step("the hero offers a download and a way back to the home page", async () => {
  const download = page.locator("#hero-download");
  if ((await download.count()) === 0) throw new Error("no hero download button");
  if (!/github\.com\/kingnazz\/ArcScan\/releases/.test(await download.getAttribute("href"))) {
    throw new Error("the hero download does not point at the releases");
  }
  const back = page.locator('.hero a[href="./"]');
  if ((await back.count()) === 0) throw new Error("no link back to the home page");
  return (await back.first().innerText()).trim();
});

await step("every required section is present and explained", async () => {
  const sections = {
    // Each section is asserted on its claims rather than on its wording, so the
    // copy can be edited without the test turning into a transcript of it.
    portable: [
      /disposable afterwards/i,
      /field session/i,
      /system temp.*arcscanportable.*sessions/i,
      /arcscan\.db/i,
      /webview/i,
      /removed after normal shutdown/i,
      /strict namespace, ownership, layout/i,
      /export csv, json or xml/i,
    ],
    choose: [
      /installer/i,
      /portable/i,
      /persistent history/i,
      /settings retained/i,
      /user exports retained/i,
      /no portable macos build/i,
    ],
    data: [
      /temporary session/i,
      /system temp.*arcscanportable.*sessions/i,
      /arcscan\.db/i,
      /webview/i,
      /validated ownership marker/i,
      /active-session lock/i,
      /csv, json and xml/i,
      /stale sessions/i,
      /unknown contents.*arbitrary temp directories are refused/i,
      /arcscan does not own/i,
    ],
    independent: [
      /installed and portable can run together/i,
      /multiple portable processes can run together/i,
      /never reads or writes those locations/i,
      /different session identifier, sqlite file, webview profile/i,
      /same extracted folder can run at the same time/i,
      /nothing is merged in either direction/i,
    ],
    updating: [
      /intentionally manual/i,
      /does not contain the nsis installer updater/i,
      /export what matters/i,
      /finish and close the session/i,
      /download the latest zip/i,
      /fresh private session/i,
      /latest\.json.*installed-updater-only/i,
      /installed.*updater is unchanged/i,
    ],
    limits: [
      /extracted folder may be read-only/i,
      /does not need to be writable/i,
      /system temporary storage is required/i,
      /never falls back to installed appdata/i,
      /keep usb media connected/i,
      /extract the zip before launching/i,
      /webview2/i,
      /not a claim arcscan makes|no dependencies.{0,40}not a claim/i,
      /smartscreen/i,
    ],
    unchanged: [
      /same application-data location/i,
      /same updater/i,
      /same read-only icmp and tcp scanning/i,
      /mdns and ssdp/i,
      /portable updates are manual/i,
      /no account, cloud service, telemetry or analytics|no telemetry/i,
    ],
  };
  for (const [id, patterns] of Object.entries(sections)) {
    const node = page.locator(`#${id}`);
    if ((await node.count()) === 0) throw new Error(`no #${id} section`);
    const text = await node.innerText();
    for (const pattern of patterns) {
      if (!pattern.test(text)) throw new Error(`#${id} does not cover ${pattern}`);
    }
  }
  return `${Object.keys(sections).length} sections`;
});

await step("the installer and portable comparison covers the real differences", async () => {
  const rows = await page.$$eval("#choose table.compare tbody tr", (trs) =>
    trs.map((tr) => ({
      label: tr.querySelector("th")?.textContent?.trim() ?? "",
      cells: Array.from(tr.querySelectorAll("td")).map((td) => td.textContent?.trim() ?? ""),
    })),
  );
  if (rows.length < 8) throw new Error(`the comparison has only ${rows.length} rows`);
  for (const needed of [
    /installation/i,
    /persistent history/i,
    /persistent inventory/i,
    /settings retained/i,
    /automatic installed update/i,
    /user exports retained/i,
    /platforms/i,
    /best for/i,
  ]) {
    if (!rows.some((r) => needed.test(r.label))) throw new Error(`no row for ${needed}`);
  }
  for (const row of rows) {
    if (row.cells.length !== 2) {
      throw new Error(`"${row.label}" has ${row.cells.length} cells, expected installer + portable`);
    }
  }
  // The platforms row is where a false portable-macOS claim would appear.
  const platforms = rows.find((r) => /platforms/i.test(r.label));
  if (/macos/i.test(platforms.cells[1])) {
    throw new Error(`the portable column claims macOS: ${platforms.cells[1]}`);
  }
  if (!/macos/i.test(platforms.cells[0])) {
    throw new Error("the installer column should still list macOS");
  }
  return `${rows.length} rows compared`;
});

await step("the page rejects obsolete persistent-Portable claims and other overclaims", async () => {
  // The prose, not the download cards. Card text flattens into one run in which
  // the ARM64 card's "Download portable" sits next to the macOS card's heading,
  // and that adjacency is not a claim. The cards have their own assertion above:
  // exactly the two Windows cards offer a portable option, and the macOS one
  // does not.
  const raw = (
    await page.$$eval("main section", (sections) =>
      sections
        // Both the download cards and the comparison table intentionally place
        // Installer macOS and Windows Portable text next to each other. Their
        // platform cells are asserted structurally above; flattened adjacency
        // is not prose claiming a Portable macOS build.
        .filter((section) => section.id !== "download" && section.id !== "choose")
        .map((section) => section.innerText)
        .join("\n"),
    )
  ).toLowerCase();
  // Sentences that deny a thing must not read as claiming it. "There is no
  // portable macOS build" and "no dependencies is not a claim ArcScan makes"
  // are the page being careful, so negated sentences are dropped before the
  // claim patterns run.
  const sentences = raw
    .split(/(?<=[.!?])\s+|\n+/)
    .filter((sentence) => !/\b(no|not|never|without|cannot|does not|is not)\b/.test(sentence));
  for (const [pattern, label] of [
    [/portable.{0,40}macos|macos.{0,40}portable/, "portable macOS"],
    [/zero dependencies/, "zero dependencies"],
    [/updates itself|self-updat/, "portable self-update"],
    [/code-signed with a paid|is signed with a paid/, "code signing it does not have"],
    [/arcscandata/, "a persistent ArcScanData folder"],
    [/\b(?:move|copy) the whole folder\b/, "folder-carried Portable state"],
    [/same-folder lock/, "the obsolete same-folder lock"],
    [/portable.{0,50}(?:automatic update check|checks latest\.json)/, "a Portable updater check"],
  ]) {
    if (sentences.some((sentence) => pattern.test(sentence))) {
      throw new Error(`the page claims ${label}`);
    }
  }
  // The denials themselves are checked against all main-page text, including
  // the comparison section. That section is omitted from `raw` only because
  // flattening its adjacent columns creates false prose such as "macOS ...
  // Portable"; its explicit footnote is still the right place to state that a
  // Portable macOS build does not exist.
  const allMainText = (await page.locator("main").innerText()).toLowerCase();
  for (const [pattern, label] of [
    [/no portable macos build/, "that there is no portable macOS build"],
    [/not a claim arcscan makes/, "that no-dependencies is not claimed"],
  ]) {
    if (!pattern.test(allMainText)) throw new Error(`the page does not state ${label}`);
  }
  // And it must make the honest versions of two of them.
  for (const [pattern, label] of [
    [/webview2/, "the WebView2 requirement"],
    [/arcscan does not own/, "that Windows keeps records ArcScan does not own"],
    [/not code-signed/, "the unsigned publisher status"],
  ]) {
    if (!pattern.test(raw)) throw new Error(`the page does not state ${label}`);
  }
  return "no overclaims";
});

await step("the release is not described as changing how scanning works", async () => {
  const text = (await page.locator("main").innerText()).toLowerCase();
  for (const pattern of [/new scan(ner|ning) engine/, /rewritten scanner/, /faster scans/]) {
    if (pattern.test(text)) throw new Error(`the page overstates the release: ${pattern}`);
  }
  if (!/unchanged|the same/.test(text)) {
    throw new Error("a distribution release must say what stayed the same");
  }
});

await step("every previous What's New page is still published and reachable", async () => {
  // People land on these from search results and from older release notes, so
  // none may 404, and none may be quietly overwritten with newer content. The
  // list grows by one every release, on purpose: an old page being deleted is
  // exactly the failure this catches.
  for (const link of [
    "whats-new-1.8.3.html",
    "whats-new-1.8.2.html",
    "whats-new-1.8.1.html",
    "whats-new-1.8.0.html",
  ]) {
    if ((await page.locator(`#download a[href="${link}"]`).count()) === 0) {
      throw new Error(`the 1.8.4 page does not link back to ${link}`);
    }
  }

  const expectations = [
    ["whats-new-1.8.3.html", /What's new in ArcScan 1\.8\.3$/, /1\.8\.3/, /device type/i],
    ["whats-new-1.8.2.html", /What's new in ArcScan 1\.8\.2$/, /1\.8\.2/, /mdns/i],
    ["whats-new-1.8.1.html", /What's new in ArcScan 1\.8\.1$/, /1\.8\.1/, /public ip/i],
    ["whats-new-1.8.0.html", /What's new in ArcScan 1\.8$/, /1\.8\.0/, /inventory/i],
  ];
  const previous = await context.newPage();
  for (const [file, heading, version, content] of expectations) {
    const response = await previous.goto(`${BASE}/${file}`, { waitUntil: "networkidle" });
    if (!response || !response.ok()) {
      throw new Error(`${file} returned ${response ? response.status() : "nothing"}`);
    }
    const h1 = (await previous.locator("h1").innerText()).trim();
    if (!heading.test(h1)) throw new Error(`${file}'s heading is now "${h1}"`);
    // Shared download cards show the latest release even on an older article.
    // Assert the archived article sections, not that intentionally live block.
    const body = await previous.$$eval("main section:not(#download)", (sections) =>
      sections.map((section) => section.innerText).join("\n"),
    );
    if (!version.test(body)) throw new Error(`${file} no longer names its own version`);
    if (!content.test(body)) throw new Error(`${file} lost its own content`);
    // An older page must keep describing its own release, not this one.
    if (/1\.8\.4/.test(body)) throw new Error(`${file} was rewritten to describe 1.8.4`);
  }
  await previous.close();
  return "1.8.3, 1.8.2, 1.8.1 and 1.8.0 intact";
});

await step("cleanup is ownership-gated and never expands to arbitrary temp data", async () => {
  const text = (await page.locator("#data").innerText()).toLowerCase();
  for (const [pattern, label] of [
    [/direct session child/, "a direct session child"],
    [/valid matching arcscan marker/, "a matching ownership marker"],
    [/inactive lock/, "an inactive process lock"],
    [/only the known database/, "the known-payload allowlist"],
    [/unknown contents/, "unknown contents being refused"],
    [/links, reparse points and arbitrary temp directories are refused/, "links and arbitrary temp being preserved"],
    [/preserving every session whose process is still active/, "active concurrent sessions being preserved"],
  ]) {
    if (!pattern.test(text)) throw new Error(`the page does not state ${label}`);
  }
  return "namespace, marker, lock, payload and active-session gates";
});

await step("the page avoids managed-service and monitoring language", async () => {
  const text = (await page.locator("main").innerText()).toLowerCase();
  for (const term of ["tenant", "msp", "managed service", "site management", "client site"]) {
    if (text.includes(term)) throw new Error(`the page uses "${term}"`);
  }
  // "Not continuous monitoring" is a disclaimer, not a claim, so it is allowed
  // only in that negated form.
  const monitoring = text.indexOf("continuous monitoring");
  if (monitoring >= 0 && !/not continuous monitoring/.test(text)) {
    throw new Error("the page claims continuous monitoring");
  }
});

await step("the technical release notes stay one click away", async () => {
  const links = await page.$$eval("a[href]", (nodes) =>
    nodes.map((n) => n.getAttribute("href")).filter(Boolean),
  );
  if (!links.includes("https://github.com/kingnazz/ArcScan/releases/tag/v1.8.4")) {
    throw new Error("no link to the v1.8.4 release notes on GitHub");
  }
  if (!links.includes("https://github.com/kingnazz/ArcScan/releases")) {
    throw new Error("no link to all releases");
  }
});

await step("the download section covers every platform", async () => {
  // Against a stubbed release, not the live API. These cards are rewritten at
  // runtime from whatever GitHub calls the latest release, so asserting this
  // release's version against the real API would only pass once this release
  // is published, and would fail every pre-release run in between. What
  // belongs to this pull request is that the page renders a 1.8.4 release
  // correctly, which is what the fixture pins.
  const released = await pageWithRelease(RELEASE_FIXTURE, undefined, WHATS_NEW);
  const cards = await released.$$eval("#download .dl", (els) =>
    els.map((el) => ({
      os: el.getAttribute("data-os"),
      version: el.querySelector('[data-field="version"]')?.textContent?.trim(),
      link: el.querySelector('[data-field="link"]')?.getAttribute("href"),
      hasChecksum: Boolean(el.querySelector('[data-field="checksum"]')),
    })),
  );
  const wanted = ["win-x64", "win-arm64", "mac"];
  for (const os of wanted) {
    const card = cards.find((c) => c.os === os);
    if (!card) throw new Error(`no download card for ${os}`);
    if (card.version !== "1.8.4") throw new Error(`${os} shows version ${card.version}`);
    if (!card.link?.startsWith("https://github.com/")) throw new Error(`${os} has no link`);
    if (!card.hasChecksum) throw new Error(`${os} has no checksum link`);
  }
  await released.close();

  // And the version the page ships, which is what a visitor sees before the
  // API answers and what scripts/sync-version.mjs keeps in step, is checked
  // from the served markup rather than from the filled-in DOM.
  const markup = await (await fetch(`${BASE}${WHATS_NEW}`)).text();
  const fallbacks = [...markup.matchAll(/<dd data-field="version">([^<]+)<\/dd>/g)].map(
    (m) => m[1].trim(),
  );
  if (fallbacks.length !== 3) throw new Error(`${fallbacks.length} static version fallbacks`);
  const stale = fallbacks.filter((v) => v !== "1.8.4");
  if (stale.length > 0) throw new Error(`static fallback still says ${stale.join(", ")}`);

  return `${cards.length} cards, all at 1.8.4, static fallback 1.8.4`;
});

await step("the portable screenshot loads at its stated size with real alt text", async () => {
  const shots = ["assets/shots/settings-portable-dark.webp"];
  for (const src of shots) {
    const img = page.locator(`img[src="${src}"]`);
    if ((await img.count()) === 0) throw new Error(`${src} is not on the page`);
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
      natural: el.naturalWidth,
      w: el.getAttribute("width"),
      h: el.getAttribute("height"),
      alt: el.getAttribute("alt") ?? "",
      srcset: el.getAttribute("srcset") ?? "",
    }));
    if (info.natural === 0) throw new Error(`${src} did not load`);
    // Intrinsic dimensions are what stop the section shifting as it loads.
    if (!info.w || !info.h) throw new Error(`${src} has no width/height attributes`);
    if (info.alt.trim().length < 60) throw new Error(`${src} needs descriptive alt text`);
    // The responsive sources are the existing optimised assets, not a reshoot.
    if (!/-800\.webp|@2x\.webp/.test(info.srcset)) {
      throw new Error(`${src} does not use the responsive assets`);
    }
    // The portable panel is shown with a fictional path, so no published image
    // of it can carry a real person's user name.
    if (/users\\\\|\/home\/|\/Users\//.test(info.alt)) {
      throw new Error(`${src} alt text describes a real-looking user path`);
    }
  }
  return `${shots.length} screenshots`;
});

await step("the What's New page adds no new stylesheet, script or font", async () => {
  const assets = await page.evaluate(() => ({
    styles: Array.from(document.querySelectorAll('link[rel="stylesheet"]')).map((n) => n.getAttribute("href")),
    scripts: Array.from(document.querySelectorAll("script[src]")).map((n) => n.getAttribute("src")),
    inline: document.querySelectorAll("script:not([src]):not([type='application/ld+json'])").length,
    fonts: Array.from(document.querySelectorAll('link[rel="preconnect"], link[href*="fonts."]')).length,
  }));
  if (assets.styles.join() !== "arcscan.v5.css") {
    throw new Error(`unexpected stylesheets: ${assets.styles.join(", ")}`);
  }
  if (assets.scripts.join() !== "arcscan.v5.js") {
    throw new Error(`unexpected scripts: ${assets.scripts.join(", ")}`);
  }
  if (assets.inline > 0) throw new Error(`${assets.inline} inline scripts`);
  if (assets.fonts > 0) throw new Error("an external font was linked");
  return "one shared stylesheet, one shared script";
});

await step("heading levels on the What's New page never skip", async () => {
  const levels = await page.$$eval("h1, h2, h3, h4", (nodes) =>
    nodes.map((n) => Number(n.tagName[1])),
  );
  const h1s = levels.filter((l) => l === 1).length;
  if (h1s !== 1) throw new Error(`expected 1 h1, found ${h1s}`);
  for (let i = 1; i < levels.length; i++) {
    if (levels[i] > levels[i - 1] + 1) {
      throw new Error(`heading jumps from h${levels[i - 1]} to h${levels[i]}`);
    }
  }
  return `${levels.length} headings`;
});

await step("no duplicate element ids on the What's New page", async () => {
  const duplicates = await page.evaluate(() => {
    const seen = new Map();
    for (const el of document.querySelectorAll("[id]")) seen.set(el.id, (seen.get(el.id) ?? 0) + 1);
    return [...seen.entries()].filter(([, n]) => n > 1).map(([id]) => id);
  });
  if (duplicates.length) throw new Error(`duplicate ids: ${duplicates.join(", ")}`);
});

await step("the What's New page is keyboard navigable from the skip link", async () => {
  await page.keyboard.press("Tab");
  const first = await page.evaluate(() => {
    const el = document.activeElement;
    return { text: el?.textContent?.trim(), href: el?.getAttribute("href") };
  });
  if (first.href !== "#main") throw new Error(`first tab stop is ${first.text}`);

  // Every interactive control has to be reachable and show a visible focus ring.
  const reachable = await page.evaluate(() => {
    const focusable = Array.from(
      document.querySelectorAll("a[href], button:not([disabled])"),
    ).filter((el) => el.offsetParent !== null);
    let ringed = 0;
    for (const el of focusable) {
      el.focus();
      const style = getComputedStyle(el);
      const outline = style.outlineStyle !== "none" && parseFloat(style.outlineWidth) > 0;
      if (outline || style.boxShadow !== "none") ringed += 1;
    }
    return { total: focusable.length, ringed };
  });
  if (reachable.total === 0) throw new Error("nothing focusable on the page");
  if (reachable.ringed < reachable.total) {
    throw new Error(`${reachable.total - reachable.ringed} controls have no visible focus state`);
  }
  return `${reachable.total} focusable controls, all with a focus ring`;
});

await step("no em dashes in the What's New copy", async () => {
  const found = await page.evaluate(() => {
    const text = document.body.innerText;
    const index = text.indexOf("\u2014");
    return index >= 0 ? text.slice(Math.max(0, index - 40), index + 40) : null;
  });
  if (found) throw new Error(`em dash found near: ${found}`);
});

await step("no horizontal overflow on the What's New page at any width", async () => {
  const problems = [];
  for (const width of WIDTHS) {
    await page.setViewportSize({ width, height: 900 });
    await page.waitForTimeout(140);
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    if (overflow > 0) problems.push(`${width}px overflows by ${overflow}px`);
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  if (problems.length) throw new Error(problems.join("; "));
  return `${WIDTHS.length} widths clean`;
});

await step("the mobile menu works on the What's New page", async () => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.reload({ waitUntil: "networkidle" });
  const toggle = page.locator("#nav-toggle");
  const panel = page.locator("#nav-panel");
  if (!(await toggle.isVisible())) throw new Error("no menu button at 390px");

  await toggle.click();
  await page.waitForTimeout(160);
  if (!(await panel.isVisible())) throw new Error("the menu did not open");
  const box = await panel.boundingBox();
  if (!box || box.x < 0 || box.x + box.width > 390 + 1) {
    throw new Error(`the menu escapes the viewport: x=${box?.x} w=${box?.width}`);
  }

  await page.keyboard.press("Escape");
  await page.waitForTimeout(160);
  if (await panel.isVisible()) throw new Error("Escape did not close the menu");

  await page.setViewportSize({ width: 1440, height: 900 });
  return `menu is ${Math.round(box.height)}px tall at 390px`;
});

await step("the home page and the What's New page reach each other", async () => {
  await page.goto(`${BASE}${WHATS_NEW}`, { waitUntil: "networkidle" });
  await page.locator('.hero a[href="./"]').first().click();
  await page.waitForLoadState("networkidle");
  if (!/See every device/.test(await page.locator("h1").innerText())) {
    throw new Error("the back link did not reach the home page");
  }
  await page.locator("#release-notes-link").click();
  await page.waitForLoadState("networkidle");
  const heading = await page.locator("h1").innerText();
  if (!/What's new in ArcScan 1\.8/.test(heading)) {
    throw new Error(`the What changed link landed on: ${heading}`);
  }
  await page.goto(BASE, { waitUntil: "networkidle" });
  return "round trip works in both directions";
});

// --- axe-core --------------------------------------------------------------
await step("axe-core finds no violations on any page", async () => {
  let AxeBuilder;
  try {
    AxeBuilder = (await import("@axe-core/playwright")).default;
  } catch {
    throw new Error("@axe-core/playwright is not installed; run npm i --no-save @axe-core/playwright");
  }
  const tags = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];
  const results = [];

  // Desktop and phone widths for the home page and the current release page,
  // plus privacy and the immediately previous What's New page. Keeping 1.8.4
  // in this matrix prevents a new release page from accidentally inheriting an
  // accessibility pass that only ever exercised old markup.
  const passes = [
    { label: "home desktop", path: "/", width: 1440, height: 900 },
    { label: "home mobile", path: "/", width: 390, height: 844 },
    { label: "privacy", path: "/privacy.html", width: 1440, height: 900 },
    { label: "whats-new 1.8.4 desktop", path: "/whats-new-1.8.4.html", width: 1440, height: 900 },
    { label: "whats-new 1.8.4 mobile", path: "/whats-new-1.8.4.html", width: 390, height: 844 },
    // The previous release's page stays published, so it stays checked.
    { label: "whats-new 1.8.3", path: "/whats-new-1.8.3.html", width: 1440, height: 900 },
  ];

  for (const { label, path, width, height } of passes) {
    await page.setViewportSize({ width, height });
    await page.goto(`${BASE}${path}`, { waitUntil: "networkidle" });

    // The switcher's other views are only in the tree once selected, so scan
    // the page again with a non-default tab open.
    if (path === "/") {
      const tab = page.locator("#tab-partial");
      if (await tab.count()) {
        await tab.click();
        await page.waitForTimeout(250);
      }
    }

    const scan = await new AxeBuilder({ page }).withTags(tags).analyze();
    if (scan.violations.length > 0) {
      throw new Error(
        scan.violations.map((v) => `${label} ${v.id} (${v.nodes.length}): ${v.help}`).join("; "),
      );
    }
    results.push(`${label}: ${scan.passes.length} checks passed`);
  }
  await page.setViewportSize({ width: 1440, height: 900 });
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
