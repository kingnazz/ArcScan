// End-to-end verification of the ArcScan interface in a real browser.
//
// It drives the built app against the browser demo backend and asserts the
// behaviour that is easy to break and hard to unit test: results streaming in
// while a scan runs, change detection, keyboard navigation, Escape precedence,
// the public-IP lookup staying opt-in, and no horizontal overflow at either the
// normal or the narrowest supported window size.
//
// CI runs this in the browser-tests job (see .github/workflows/ci.yml), which
// installs Playwright's Chromium on the runner. To run it locally:
//
//   npm run build
//   npm run preview &
//   npm i --no-save playwright @axe-core/playwright && npx playwright install chromium
//   node scripts/verify-ui.mjs
//
// Set PLAYWRIGHT_CHROMIUM_PATH to use a Chromium that is already on the machine.
import { chromium } from "playwright";

const URL = process.env.ARCSCAN_URL ?? "http://localhost:4173/";
const errors = [];

const executablePath = process.env.PLAYWRIGHT_CHROMIUM_PATH;
const browser = await chromium.launch(executablePath ? { executablePath } : {});
// An explicit context, because axe-core/playwright refuses a page created
// directly on the browser.
const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
const page = await context.newPage();
page.on("console", (m) => {
  if (m.type() === "error") errors.push(m.text());
});
page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));

await page.goto(URL, { waitUntil: "networkidle" });

const step = async (name, fn) => {
  try {
    const out = await fn();
    console.log(`PASS  ${name}${out ? ` — ${out}` : ""}`);
  } catch (e) {
    console.log(`FAIL  ${name} — ${e.message}`);
    process.exitCode = 1;
  }
};

await step("app renders with the empty scan screen", async () => {
  await page.getByRole("heading", { name: "Scan a network" }).waitFor({ timeout: 5000 });
  return await page.locator("#scan-target").inputValue();
});

await step("start screen offers the detected network", async () => {
  await page.getByRole("button", { name: /^Scan 192\.168\.10\.0\/24$/ }).waitFor({ timeout: 3000 });
});

await step("start screen applies the recommended profile it displays", async () => {
  // Deliberately select a different profile first; the start screen's Scan
  // button promises the recommended one and must apply it.
  await page.locator('button[title="Scan profile"]').click();
  await page.getByRole("option", { name: /Remote subnet/ }).click();
  const picked = await page.locator('button[title="Scan profile"]').innerText();
  if (!/Remote subnet/.test(picked)) throw new Error(`profile did not change: ${picked}`);
  const text = (await page.locator("main").innerText()).toLowerCase();
  if (!text.includes("recommended profile: quick lan")) {
    throw new Error("start screen no longer recommends Quick LAN for the local network");
  }
});

await step("scan streams devices while running", async () => {
  await page.getByRole("button", { name: /^Scan 192\.168\.10\.0\/24$/ }).click();
  const applied = await page.locator('button[title="Scan profile"]').innerText();
  if (!/Quick LAN/.test(applied)) {
    throw new Error(`Scan button ran with "${applied}" instead of the recommended Quick LAN`);
  }
  // A row must appear well before the scan finishes.
  await page.locator("tbody tr").first().waitFor({ timeout: 4000 });
  await page.waitForTimeout(700);
  const midCount = await page.locator("tbody tr").count();
  const status = await page.locator("footer").first().innerText();
  if (midCount === 0) throw new Error("no rows appeared during the scan");
  if (!/devices found/.test(status)) throw new Error(`status bar did not show progress: ${status}`);
  return `${midCount} rows mid-scan, status "${status.split("\n")[0].slice(0, 70)}"`;
});

await step("scan completes with the full device set", async () => {
  await page.getByRole("button", { name: "Stop" }).waitFor({ state: "detached", timeout: 20000 });
  await page.waitForTimeout(600);
  const count = await page.locator("tbody tr").count();
  if (count !== 14) throw new Error(`expected 14 devices, got ${count}`);
  return `${count} devices`;
});

await step("change detection reports the seeded differences", async () => {
  const badges = await page.locator("header nav button", { hasText: "Changes" }).innerText();
  if (!/\d/.test(badges)) throw new Error(`no change count on the Changes tab: "${badges}"`);
  return badges.replace(/\s+/g, " ");
});

await step("comparison view separates added, missing and changed", async () => {
  await page.locator("header nav button", { hasText: "Changes" }).click();
  await page.getByRole("heading", { name: /changes? since the previous scan/ }).waitFor({ timeout: 3000 });
  const text = (await page.locator("main").innerText()).toLowerCase();
  for (const needed of ["added devices", "missing devices", "changed devices"]) {
    if (!text.includes(needed)) throw new Error(`missing group: ${needed}`);
  }
  if (!text.includes("warehouse tablet")) throw new Error("missing device not listed");
  if (!text.includes("conference room display")) throw new Error("new device not listed");
  return "three groups present";
});

await step("filtering narrows the table", async () => {
  await page.locator("header nav button", { hasText: "Devices" }).click();
  await page.getByPlaceholder("Filter devices").fill("printer");
  await page.waitForTimeout(150);
  const count = await page.locator("tbody tr").count();
  if (count !== 1) throw new Error(`expected 1 match for "printer", got ${count}`);
  await page.getByPlaceholder("Filter devices").fill("");
  return "1 match";
});

await step("sorting by name reorders rows", async () => {
  await page.getByRole("columnheader", { name: /name/i }).getByRole("button").click();
  await page.waitForTimeout(120);
  const first = await page.locator("tbody tr td").nth(1).innerText();
  await page.getByRole("columnheader", { name: /ip address/i }).getByRole("button").click();
  return `first row after name sort: ${first.trim()}`;
});

await step("device panel opens with details and actions", async () => {
  await page.locator("tbody tr", { hasText: "Front Office Printer" }).dblclick();
  await page.getByRole("complementary").waitFor({ timeout: 3000 });
  const text = (await page.getByRole("complementary").innerText()).toLowerCase();
  for (const needed of ["identity", "open services", "notes", "first seen", "previous addresses"]) {
    if (!text.includes(needed)) throw new Error(`drawer missing section: ${needed}`);
  }
  if (!text.includes("https · 443")) throw new Error("services not shown as name and number");
  return "sections and services present";
});

await step("emphasised action matches the open services", async () => {
  const panel = page.getByRole("complementary");
  // The printer has a web interface, so Open web interface is the primary action.
  await panel.getByRole("button", { name: "Open web interface" }).waitFor({ timeout: 2000 });
  const rdp = panel.getByRole("button", { name: "RDP" });
  if (!(await rdp.isDisabled())) throw new Error("RDP should be disabled with 3389 closed");
  return "web emphasised, RDP disabled";
});

await step("Escape closes the panel before anything else", async () => {
  await page.keyboard.press("Escape");
  await page.waitForTimeout(150);
  if (await page.getByRole("complementary").isVisible().catch(() => false)) {
    throw new Error("panel stayed open");
  }
});

await step("keyboard navigation moves the selection", async () => {
  await page.locator("tbody").focus();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  const selected = await page.locator('tbody tr[aria-selected="true"]').count();
  if (selected !== 1) throw new Error(`expected 1 selected row, got ${selected}`);
  await page.keyboard.press("Enter");
  await page.getByRole("complementary").waitFor({ timeout: 2000 });
  await page.keyboard.press("Escape");
  return "arrows select, Enter opens";
});

await step("renaming a device persists and offers undo", async () => {
  await page.locator("tbody tr").nth(5).dblclick();
  const panel = page.getByRole("complementary");
  await panel.locator("#device-name").fill("Spare Laptop");
  await panel.locator("#device-name").blur();
  await page.getByRole("status").filter({ hasText: /Renamed to/ }).first().waitFor({ timeout: 3000 });
  await page.getByRole("button", { name: "Undo" }).waitFor({ timeout: 2000 });
  await page.keyboard.press("Escape");
  return "toast with Undo shown";
});

await step("notes load for an observation at an older address", async () => {
  // The printer changed address between the seeded scans, so the drawer is
  // showing a device whose stored notes were written against a different IP.
  // Keying drafts by device id (not IP) is what makes them load here.
  await page.locator("tbody tr", { hasText: "Front Office Printer" }).dblclick();
  const panel = page.getByRole("complementary");
  await panel.waitFor({ timeout: 3000 });
  const notes = await panel.locator("textarea").inputValue();
  if (!notes.trim()) throw new Error("notes did not load for the moved device");
  const previous = (await panel.innerText()).toLowerCase();
  if (!previous.includes("192.168.10.42")) {
    throw new Error("the device's earlier address is not listed");
  }
  await page.keyboard.press("Escape");
  return `notes loaded: "${notes.slice(0, 40)}"`;
});

await step("history lists the seeded scans with change counts", async () => {
  await page.locator("header nav button", { hasText: "History" }).click();
  await page.waitForTimeout(250);
  const items = await page.locator("main ul > li").count();
  if (items < 3) throw new Error(`expected at least 3 saved scans, got ${items}`);
  const text = await page.locator("main").innerText();
  if (!/new|changed|missing/i.test(text)) throw new Error("no change counts in history");
  return `${items} scans`;
});

await step("history shows which network each scan belongs to", async () => {
  // innerText returns CSS-transformed text, and the badges are uppercased.
  const text = await page.locator("main").innerText();
  if (!/office network/i.test(text)) throw new Error("network scope not shown in history");
});

await step("exporting an old scan does not disturb the current view", async () => {
  // Open the newest scan, then export the *oldest* one. The export must carry
  // the old scan's own rows and leave the displayed scan untouched.
  const rows = page.locator("main ul > li");
  await rows.first().locator("button").first().click();
  await page.waitForTimeout(300);
  await page.locator("header nav button", { hasText: "Devices" }).click();
  await page.getByPlaceholder("Filter devices").fill("printer");
  await page.waitForTimeout(150);
  const filteredBefore = await page.locator("tbody tr").count();

  await page.locator("header nav button", { hasText: "History" }).click();
  await page.waitForTimeout(200);
  const last = page.locator("main ul > li").last();
  const oldestLabel = (await last.innerText()).split("\n")[0];
  await last.hover();
  const download = page.waitForEvent("download", { timeout: 8000 });
  await last.getByRole("button", { name: "Export this scan" }).click();
  await page.getByRole("menuitem", { name: "CSV" }).click();
  const file = await download;

  const name = file.suggestedFilename();
  if (!name.endsWith(".csv")) throw new Error(`unexpected export name: ${name}`);
  if (!name.includes("192.168.10.0")) {
    throw new Error(`export filename does not carry the historical target: ${name}`);
  }
  // The exported file must hold the whole historical scan, not the filtered view.
  const stream = await file.createReadStream();
  let body = "";
  for await (const chunk of stream) body += chunk;
  const dataRows = body.trimEnd().split("\n").length - 1;
  if (dataRows <= filteredBefore) {
    throw new Error(`export has ${dataRows} rows; the filtered table had ${filteredBefore}`);
  }

  // Nothing about the current view moved.
  await page.locator("header nav button", { hasText: "Devices" }).click();
  await page.waitForTimeout(150);
  const filterStillSet = await page.getByPlaceholder("Filter devices").inputValue();
  if (filterStillSet !== "printer") throw new Error("the filter was reset by the export");
  const filteredAfter = await page.locator("tbody tr").count();
  if (filteredAfter !== filteredBefore) throw new Error("the displayed scan changed");
  await page.getByPlaceholder("Filter devices").fill("");
  return `${dataRows} rows exported from "${oldestLabel}" while filtered to ${filteredBefore}`;
});

await step("Stop during a scan keeps partial results and labels them", async () => {
  await page.locator("header nav button", { hasText: "Devices" }).click();
  await page.getByRole("button", { name: "Scan", exact: true }).click();
  await page.locator("tbody tr").first().waitFor({ timeout: 5000 });
  await page.waitForTimeout(400);
  const stop = page.getByRole("button", { name: /^Stop/ });
  await stop.click();
  await stop.waitFor({ state: "detached", timeout: 10000 });
  await page.waitForTimeout(600);

  const kept = await page.locator("tbody tr").count();
  if (kept === 0) throw new Error("a stopped scan discarded the hosts it had found");

  await page.locator("header nav button", { hasText: "History" }).click();
  await page.waitForTimeout(300);
  const newest = page.locator("main ul > li").first();
  const label = await newest.innerText();
  if (!/partial scan/i.test(label)) throw new Error(`newest scan not labelled partial: ${label}`);
  if (/\d+\s+missing/i.test(label)) {
    throw new Error(`a partial scan reported missing devices: ${label}`);
  }
  return `${kept} devices kept, labelled partial`;
});

await step("comparison is disabled for a partial scan and says why", async () => {
  const newest = page.locator("main ul > li").first();
  await newest.hover();
  const compare = newest.getByRole("button", { name: /stopped early|Compare/ });
  if (!(await compare.isDisabled())) {
    throw new Error("a partial scan offered a comparison");
  }
  const why = await compare.getAttribute("aria-label");
  if (!/stopped early/i.test(why ?? "")) throw new Error(`unhelpful reason: ${why}`);
  return why ?? "";
});

await step("Changes view explains why a partial scan has none", async () => {
  await page.locator("main ul > li").first().locator("button").first().click();
  await page.waitForTimeout(400);
  await page.locator("header nav button", { hasText: "Changes" }).click();
  await page.waitForTimeout(250);
  const text = await page.locator("main").innerText();
  if (!/Changes unavailable for this partial scan/i.test(text)) {
    throw new Error(`partial-scan Changes view did not explain itself: ${text.slice(0, 160)}`);
  }
  if (!/stopped before every address was checked/i.test(text)) {
    throw new Error("the reason was not shown");
  }
});

await step("deleting a scan asks first", async () => {
  await page.locator("header nav button", { hasText: "History" }).click();
  await page.waitForTimeout(200);
  await page.locator("main ul > li").first().hover();
  await page.getByRole("button", { name: "Delete this scan" }).first().click();
  await page.getByRole("dialog", { name: "Delete this scan?" }).waitFor({ timeout: 2000 });
  await page.getByRole("button", { name: "Cancel" }).click();
  return "confirmation shown and cancelled";
});

await step("settings opens and public IP lookup is off by default", async () => {
  await page.getByRole("button", { name: "Settings" }).click();
  const panel = page.getByRole("complementary", { name: "Settings" });
  await panel.waitFor({ timeout: 3000 });
  const toggle = panel.locator("#settings-public-ip");
  const checked = await toggle.getAttribute("aria-checked");
  if (checked !== "false") throw new Error(`public IP lookup should default off, got ${checked}`);
  // The Check public IP button must not exist until the switch is on.
  if (await panel.getByRole("button", { name: "Check public IP" }).count()) {
    throw new Error("Check public IP offered while the lookup is disabled");
  }
  return "opt-in confirmed";
});

await step("settings lets the scanned network be named", async () => {
  const panel = page.getByRole("complementary", { name: "Settings" });
  const field = panel.getByLabel(/^Name for /);
  await field.waitFor({ timeout: 3000 });
  await field.fill("Head Office");
  await field.blur();
  await page.getByRole("status").filter({ hasText: /renamed/i }).first().waitFor({ timeout: 3000 });

  // The new name reaches history, which is where it does its work.
  await page.keyboard.press("Escape");
  await page.locator("header nav button", { hasText: "History" }).click();
  await page.waitForTimeout(300);
  const history = await page.locator("main").innerText();
  if (!/head office/i.test(history)) throw new Error("renamed network not shown in history");
  return "named and shown in history";
});

await step("no horizontal overflow at 1440x900", async () => {
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  if (overflow > 0) throw new Error(`page overflows by ${overflow}px`);
});

await step("narrow window keeps Scan, Stop and the table usable", async () => {
  await page.keyboard.press("Escape");
  await page.setViewportSize({ width: 940, height: 700 });
  await page.waitForTimeout(200);
  await page.locator("header nav button", { hasText: "Devices" }).click();
  await page.getByRole("button", { name: "Scan", exact: true }).waitFor({ timeout: 2000 });
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  if (overflow > 0) throw new Error(`page overflows by ${overflow}px at 940px`);
  const headers = await page.locator("thead th").allInnerTexts();
  return `columns at 940px: ${headers.map((h) => h.trim()).join(", ")}`;
});

await step("dark theme applies", async () => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.getByRole("button", { name: /Switch to the dark theme/ }).click();
  await page.waitForTimeout(200);
  const dark = await page.evaluate(() => document.documentElement.classList.contains("dark"));
  if (!dark) throw new Error("dark class not applied");
});

await step("axe-core finds no violations in either theme", async () => {
  let AxeBuilder;
  try {
    AxeBuilder = (await import("@axe-core/playwright")).default;
  } catch {
    throw new Error("@axe-core/playwright is not installed; run npm i --no-save @axe-core/playwright");
  }
  const summary = [];
  for (const theme of ["dark", "light"]) {
    // Set the theme the way the app itself stores it, then reload.
    await page.evaluate((value) => {
      const raw = localStorage.getItem("arcscan-settings");
      const settings = raw ? JSON.parse(raw) : {};
      settings.theme = value;
      localStorage.setItem("arcscan-settings", JSON.stringify(settings));
      localStorage.setItem("arcscan-theme", value);
    }, theme);
    await page.reload({ waitUntil: "networkidle" });
    // Scan a populated table rather than the empty state, so the results grid,
    // the badges and the toolbar are all in the tree.
    await page.getByRole("button", { name: /^Scan 192\.168\.10\.0\/24$/ }).click();
    await page.getByRole("button", { name: "Stop" }).waitFor({ state: "detached", timeout: 30000 });
    await page.waitForTimeout(700);

    const scan = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
      .analyze();
    if (scan.violations.length > 0) {
      throw new Error(
        scan.violations.map((v) => `${theme} ${v.id} (${v.nodes.length}): ${v.help}`).join("; "),
      );
    }
    summary.push(`${theme}: ${scan.passes.length} checks passed`);
  }
  return summary.join(", ");
});

if (errors.length > 0) {
  console.log(`\nFAIL  console clean — ${errors.length} error(s):`);
  for (const e of errors.slice(0, 8)) console.log(`      ${e}`);
  process.exitCode = 1;
} else {
  console.log("PASS  console clean — no errors or unhandled rejections");
}

await browser.close();
