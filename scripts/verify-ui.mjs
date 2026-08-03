// End-to-end verification of the ArcScan interface in a real browser.
//
// It drives the built app against the browser demo backend and asserts the
// behaviour that is easy to break and hard to unit test: the persistent
// Inventory and its presence states, the Changes inbox and every action it
// offers, results streaming in while a scan runs, partial-scan safety, keyboard
// navigation, Escape precedence, filters surviving a view change, and no
// horizontal overflow at any supported width.
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
// directly on the browser. Reduced motion is on so no assertion races an
// animation, and because it is a supported mode that has to stay usable.
const context = await browser.newContext({
  viewport: { width: 1440, height: 900 },
  reducedMotion: "reduce",
});
const page = await context.newPage();
page.on("console", (m) => {
  if (m.type() === "error") errors.push(m.text());
});
page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));

const step = async (name, fn) => {
  try {
    const out = await fn();
    console.log(`PASS  ${name}${out ? ` — ${out}` : ""}`);
  } catch (e) {
    console.log(`FAIL  ${name} — ${e.message}`);
    process.exitCode = 1;
  }
};

const nav = (label) => page.locator("header nav button", { hasText: label });
const mainText = async () => (await page.locator("main").innerText()).toLowerCase();
/**
 * The Changes entries only. `main` also carries the filter menus, whose option
 * text ("Service changes", "Missing devices") would satisfy every assertion
 * about the list without a single entry being present.
 */
const inboxText = async () => {
  const list = page.locator("main ul").first();
  return (await list.count()) === 0 ? "" : (await list.innerText()).toLowerCase();
};
/** The scan button in the command bar, not the Scan tab of the same name. */
const scanButton = () => page.locator('form button[type="submit"]');

// ---------------------------------------------------------------------------
// 1. Empty states, on a demo with no history at all
// ---------------------------------------------------------------------------

await page.goto(`${URL}?demo=empty`, { waitUntil: "networkidle" });

await step("empty Inventory asks for a first scan", async () => {
  await nav("Inventory").click();
  await page.waitForTimeout(300);
  const text = await mainText();
  if (!text.includes("no inventory yet")) {
    throw new Error(`unexpected empty state: ${text.slice(0, 140)}`);
  }
  if (!text.includes("run your first scan")) {
    throw new Error("the empty state does not say what to do");
  }
  if ((await page.locator("tbody tr").count()) !== 0) throw new Error("rows in an empty inventory");
});

await step("empty Changes explains itself rather than looking broken", async () => {
  await nav("Changes").click();
  await page.waitForTimeout(200);
  const text = await mainText();
  if (!text.includes("no changes recorded yet")) throw new Error(text.slice(0, 140));
  if (!text.includes("two scans")) throw new Error("the empty inbox does not explain what fills it");
});

// ---------------------------------------------------------------------------
// 2. The seeded demo: inventory, networks, changes
// ---------------------------------------------------------------------------

await page.goto(URL, { waitUntil: "networkidle" });

await step("app renders with the scan screen and the detected network", async () => {
  await page.getByRole("heading", { name: "Scan a network" }).waitFor({ timeout: 5000 });
  await page.getByRole("button", { name: /^Scan 192\.168\.1\.0\/24$/ }).waitFor({ timeout: 3000 });
  return await page.locator("#scan-target").inputValue();
});

await step("Inventory lists devices across scans with a compact summary", async () => {
  await nav("Inventory").click();
  await page.locator("tbody tr").first().waitFor({ timeout: 4000 });
  const rows = await page.locator("tbody tr").count();
  const text = await mainText();
  const headline = text.match(/(\d+) devices · (\d+) present · (\d+) missing · (\d+) unknown/);
  if (!headline) throw new Error(`no compact summary: ${text.slice(0, 160)}`);
  if (Number(headline[1]) !== rows) {
    throw new Error(`summary says ${headline[1]} devices, table shows ${rows}`);
  }
  // All three presence states must be represented, or the demo is not
  // exercising the rules this release is about.
  for (const group of [2, 3, 4]) {
    if (Number(headline[group]) === 0) throw new Error(`nothing in state ${group}: ${headline[0]}`);
  }
  return headline[0];
});

await step("presence is never described as online", async () => {
  const text = await mainText();
  if (/\bonline\b/.test(text)) throw new Error("the inventory claims a device is online");
  if (!text.includes("present")) throw new Error("presence is not shown");
});

await step("Inventory search narrows the table", async () => {
  await page.getByPlaceholder("Search inventory").fill("printer");
  await page.waitForTimeout(200);
  const count = await page.locator("tbody tr").count();
  if (count < 1 || count > 2) throw new Error(`expected the printers, got ${count} rows`);
  // Search reaches a previous address as well as the current one.
  await page.getByPlaceholder("Search inventory").fill("192.168.1.28");
  await page.waitForTimeout(200);
  if ((await page.locator("tbody tr").count()) !== 1) {
    throw new Error("searching a previous address found nothing");
  }
  await page.getByPlaceholder("Search inventory").fill("");
  return "name and previous address both match";
});

await step("the Missing filter shows only devices absent from the latest scan", async () => {
  await page.getByLabel("Filter the inventory").selectOption("missing");
  await page.waitForTimeout(200);
  const rows = await page.locator("tbody tr").count();
  if (rows === 0) throw new Error("no missing devices in a demo that has one");
  const states = await page.locator("tbody tr td:nth-child(4)").allInnerTexts();
  if (states.some((s) => !/missing/i.test(s))) {
    throw new Error(`a non-missing device survived the filter: ${states.join(", ")}`);
  }
  return `${rows} missing`;
});

await step("the Unknown filter reaches devices presence cannot be decided for", async () => {
  await page.getByLabel("Filter the inventory").selectOption("unknown");
  await page.waitForTimeout(200);
  if ((await page.locator("tbody tr").count()) === 0) {
    throw new Error("no unknown devices in a demo that has one");
  }
  await page.getByLabel("Filter the inventory").selectOption("all");
});

await step("multiple networks each get a filter and stay separate", async () => {
  const filter = page.getByLabel("Filter by network");
  await filter.waitFor({ timeout: 3000 });
  const options = await filter.locator("option").allInnerTexts();
  if (options.length < 3) {
    throw new Error(`expected all-networks plus two, got ${options.join(", ")}`);
  }
  const office = options.find((o) => /office/i.test(o));
  if (!office) throw new Error(`no office network: ${options.join(", ")}`);

  await filter.selectOption({ label: office });
  await page.waitForTimeout(250);
  const networks = await page.locator("tbody tr td:nth-child(7)").allInnerTexts();
  const distinct = [...new Set(networks.map((n) => n.trim()).filter(Boolean))];
  if (distinct.length !== 1) throw new Error(`the network filter mixed ${distinct.join(", ")}`);
  await filter.selectOption("");
  return `${options.length - 1} networks, filtered to ${distinct[0]}`;
});

await step("filters survive a trip to another view", async () => {
  await page.getByPlaceholder("Search inventory").fill("nas");
  await page.getByLabel("Filter the inventory").selectOption("present");
  await page.waitForTimeout(200);
  const before = await page.locator("tbody tr").count();

  await nav("History").click();
  await page.waitForTimeout(250);
  await nav("Inventory").click();
  await page.waitForTimeout(250);

  if ((await page.getByPlaceholder("Search inventory").inputValue()) !== "nas") {
    throw new Error("the search was cleared by switching views");
  }
  if ((await page.locator("tbody tr").count()) !== before) {
    throw new Error("the filtered set changed across a view switch");
  }
  await page.getByPlaceholder("Search inventory").fill("");
  await page.getByLabel("Filter the inventory").selectOption("all");
  return `${before} rows preserved`;
});

// ---------------------------------------------------------------------------
// 3. Selection, bulk actions and exports
// ---------------------------------------------------------------------------

await step("selecting rows reveals the bulk actions and a count", async () => {
  await page.locator('tbody input[type="checkbox"]').first().check();
  await page.locator('tbody input[type="checkbox"]').nth(1).check();
  const toolbar = page.getByRole("toolbar", { name: "Actions for the selected devices" });
  await toolbar.waitFor({ timeout: 2000 });
  const text = await toolbar.innerText();
  if (!/2 devices selected/i.test(text)) throw new Error(`unexpected count: ${text}`);
  for (const action of ["Mark trusted", "Mark unreviewed", "Ignore", "Copy addresses"]) {
    if (!text.includes(action)) throw new Error(`missing bulk action: ${action}`);
  }
  // Nothing destructive is offered.
  if (/delete|remove/i.test(text)) throw new Error("a destructive bulk action is offered");
  return "count and actions shown";
});

await step("a bulk classification applies and clears the selection", async () => {
  await page.getByRole("button", { name: "Mark trusted" }).click();
  await page
    .getByRole("status")
    .filter({ hasText: /Marked trusted/ })
    .first()
    .waitFor({ timeout: 4000 });
  await page.waitForTimeout(400);
  const stillSelected = await page
    .getByRole("toolbar", { name: "Actions for the selected devices" })
    .isVisible()
    .catch(() => false);
  if (stillSelected) throw new Error("the selection survived a successful action");

  await page.getByLabel("Filter the inventory").selectOption("trusted");
  await page.waitForTimeout(250);
  const trusted = await page.locator("tbody tr").count();
  await page.getByLabel("Filter the inventory").selectOption("all");
  if (trusted < 2) throw new Error(`expected at least 2 trusted devices, got ${trusted}`);
  return `${trusted} trusted`;
});

await step("Inventory export writes the whole filtered set with real headers", async () => {
  const download = page.waitForEvent("download", { timeout: 8000 });
  await page.getByRole("button", { name: "Export", exact: true }).click();
  await page.getByRole("button", { name: "CSV spreadsheet" }).click();
  const file = await download;

  const name = file.suggestedFilename();
  if (!/^arcscan-inventory-\d{4}-\d{2}-\d{2}\.csv$/.test(name)) {
    throw new Error(`unexpected export name: ${name}`);
  }
  const stream = await file.createReadStream();
  let body = "";
  for await (const chunk of stream) body += chunk;
  const lines = body.trimEnd().split("\n");
  if (!lines[0].startsWith("Network,Device,Status,Presence,")) {
    throw new Error(`unexpected header: ${lines[0]}`);
  }
  const rows = await page.locator("tbody tr").count();
  if (lines.length - 1 !== rows) {
    throw new Error(`exported ${lines.length - 1} rows for a table of ${rows}`);
  }
  if (!/Present in latest scan|Missing from latest scan/.test(body)) {
    throw new Error("presence is not spelled out in the export");
  }
  return `${lines.length - 1} rows as ${name}`;
});

await step("exporting one network names it in the file", async () => {
  const filter = page.getByLabel("Filter by network");
  const options = await filter.locator("option").allInnerTexts();
  await filter.selectOption({ label: options.find((o) => /office/i.test(o)) });
  await page.waitForTimeout(250);

  const download = page.waitForEvent("download", { timeout: 8000 });
  await page.getByRole("button", { name: "Export", exact: true }).click();
  await page.getByRole("button", { name: "CSV spreadsheet" }).click();
  const name = (await download).suggestedFilename();
  await filter.selectOption("");
  if (!name.includes("office")) throw new Error(`the scope is not in the filename: ${name}`);
  return name;
});

// ---------------------------------------------------------------------------
// 4. The device drawer from the Inventory
// ---------------------------------------------------------------------------

await step("the drawer opens from the Inventory with the persistent facts", async () => {
  await page.locator("tbody tr", { hasText: "Office Printer" }).first().dblclick();
  const panel = page.getByRole("complementary");
  await panel.waitFor({ timeout: 3000 });
  const text = (await panel.innerText()).toLowerCase();
  for (const needed of [
    "present in latest scan",
    "network",
    "first seen",
    "last seen",
    "observations",
    "previous addresses",
    "recorded changes",
    "notes",
    "scan history",
  ]) {
    if (!text.includes(needed)) throw new Error(`drawer missing: ${needed}`);
  }
  if (text.includes("no icmp reply")) {
    throw new Error("the drawer claims no reply for a device that answered");
  }
  return "presence, network and history present";
});

await step("renaming from the Inventory persists and offers undo", async () => {
  const panel = page.getByRole("complementary");
  await panel.locator("#device-name").fill("Upstairs Printer");
  await panel.locator("#device-name").blur();
  await page
    .getByRole("status")
    .filter({ hasText: /Renamed to/ })
    .first()
    .waitFor({ timeout: 4000 });
  await page.getByRole("button", { name: "Undo" }).waitFor({ timeout: 2000 });
  await page.waitForTimeout(500);
  if (!(await mainText()).includes("upstairs printer")) {
    throw new Error("the inventory kept the old name");
  }
  return "rename reached the table";
});

await step("notes save and reload for a device whose address changed", async () => {
  const panel = page.getByRole("complementary");
  const notes = panel.locator("textarea");
  const existing = await notes.inputValue();
  if (!existing.trim()) throw new Error("stored notes did not load");
  await notes.fill(`${existing} Checked today.`);
  await notes.blur();
  await page
    .getByRole("status")
    .filter({ hasText: /Notes saved/ })
    .first()
    .waitFor({ timeout: 4000 });
  await page.keyboard.press("Escape");
  await page.waitForTimeout(250);

  await page.locator("tbody tr", { hasText: "Upstairs Printer" }).first().dblclick();
  await page.getByRole("complementary").waitFor({ timeout: 3000 });
  await page.waitForTimeout(300);
  const reloaded = await page.getByRole("complementary").locator("textarea").inputValue();
  if (!reloaded.includes("Checked today.")) throw new Error("notes did not survive a reopen");
  await page.keyboard.press("Escape");
  return "notes round-tripped across an address change";
});

// ---------------------------------------------------------------------------
// 5. Keyboard and Escape precedence
// ---------------------------------------------------------------------------

await step("arrow keys select, Space toggles and Enter opens", async () => {
  await page.locator("tbody").focus();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  if ((await page.locator('tbody tr[aria-selected="true"]').count()) !== 1) {
    throw new Error("arrow keys did not move a single selection");
  }
  await page.keyboard.press(" ");
  await page.waitForTimeout(200);
  if ((await page.locator('tbody input[type="checkbox"]:checked').count()) !== 1) {
    throw new Error("Space did not tick the focused row");
  }
  await page.keyboard.press("Enter");
  await page.getByRole("complementary").waitFor({ timeout: 2000 });
  return "arrows, Space and Enter all work";
});

await step("Escape closes the drawer, then the selection, then the filters", async () => {
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  if (await page.getByRole("complementary").isVisible().catch(() => false)) {
    throw new Error("the drawer stayed open");
  }
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  if ((await page.locator('tbody input[type="checkbox"]:checked').count()) !== 0) {
    throw new Error("Escape did not clear the selection");
  }
  await page.getByPlaceholder("Search inventory").fill("printer");
  await page.locator("tbody").focus();
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  if ((await page.getByPlaceholder("Search inventory").inputValue()) !== "") {
    throw new Error("Escape did not clear the search");
  }
  return "drawer, selection, filters, in that order";
});

// ---------------------------------------------------------------------------
// 6. The Changes inbox
// ---------------------------------------------------------------------------

await step("the inbox lists every major kind of change", async () => {
  await nav("Changes").click();
  await page.locator("main ul > li").first().waitFor({ timeout: 4000 });
  await page.getByLabel("Filter changes", { exact: true }).selectOption("all");
  await page.waitForTimeout(250);
  const text = await inboxText();
  for (const needed of [
    "new device",
    "missing",
    "address change",
    "service change",
    "hostname change",
  ]) {
    if (!text.includes(needed)) throw new Error(`no ${needed} in the inbox`);
  }
  if (!text.includes("opened:")) {
    throw new Error("port changes are not shown as opened and closed services");
  }
  if (/\boffline\b/.test(text)) throw new Error("the inbox claims a device is offline");
  return "added, missing, address, service and name changes present";
});

await step("the network badge tells the two networks apart", async () => {
  const text = await inboxText();
  if (!/home wi-fi/i.test(text) || !/office/i.test(text)) {
    throw new Error("changes are not attributed to their network");
  }
});

await step("the change-type filter narrows to one kind", async () => {
  await page.getByLabel("Filter changes", { exact: true }).selectOption("added");
  await page.waitForTimeout(250);
  const items = await page.locator("main ul > li").count();
  if (items === 0) throw new Error("no new-device entries");
  if ((await inboxText()).includes("service change")) {
    throw new Error("a service change survived the New devices filter");
  }
  await page.getByLabel("Filter changes", { exact: true }).selectOption("all");
  return `${items} new devices`;
});

await step("ignored entries stay out of every view except Ignored", async () => {
  if (/driveway camera/.test(await inboxText())) {
    throw new Error("an ignored device's change is in All changes");
  }
  await page.getByLabel("Filter changes", { exact: true }).selectOption("ignored");
  await page.waitForTimeout(250);
  if (!/driveway camera/.test(await inboxText())) {
    throw new Error("the Ignored filter does not bring it back");
  }
  await page.getByLabel("Filter changes", { exact: true }).selectOption("unreviewed");
  return "hidden by default, reachable on demand";
});

await step("Trust classifies the device and acknowledges only that entry", async () => {
  const before = await page.locator("main ul > li").count();
  const entry = page.locator("main ul > li", { hasText: "New device" }).first();
  await entry.getByRole("button", { name: "Trust" }).click();
  await page
    .getByRole("status")
    .filter({ hasText: /Trusted and acknowledged/ })
    .first()
    .waitFor({ timeout: 5000 });
  await page.waitForTimeout(500);
  const after = await page.locator("main ul > li").count();
  if (after >= before) throw new Error("the trusted entry stayed in the unreviewed inbox");

  // The record is kept, not deleted.
  await page.getByLabel("Filter changes", { exact: true }).selectOption("acknowledged");
  await page.waitForTimeout(250);
  if ((await page.locator("main ul > li").count()) === 0) {
    throw new Error("acknowledging deleted the record");
  }
  await page.getByLabel("Filter changes", { exact: true }).selectOption("unreviewed");
  return `${before} to ${after} unreviewed`;
});

await step("Acknowledge can be undone", async () => {
  const before = await page.locator("main ul > li").count();
  await page
    .locator("main ul > li")
    .first()
    .getByRole("button", { name: "Acknowledge", exact: true })
    .click();
  await page
    .getByRole("status")
    .filter({ hasText: /Acknowledged\./ })
    .first()
    .waitFor({ timeout: 5000 });
  await page.getByRole("button", { name: "Undo" }).last().click();
  await page.waitForTimeout(700);
  const after = await page.locator("main ul > li").count();
  if (after !== before) throw new Error(`undo left ${after} entries where there were ${before}`);
  return "reopened";
});

await step("Acknowledge visible clears exactly what is on screen", async () => {
  const unreviewed = async () => Number((await mainText()).match(/(\d+) unreviewed/)?.[1] ?? -1);
  const total = await unreviewed();
  if (total < 2) throw new Error(`needs more than one unreviewed change, has ${total}`);

  await page.getByPlaceholder("Search changes").fill("printer");
  await page.waitForTimeout(300);
  const groups = await page.locator("main ul > li").count();
  if (groups === 0) throw new Error("the search matched nothing to acknowledge");

  await page.getByRole("button", { name: "Acknowledge visible" }).click();
  await page.waitForTimeout(900);
  const remaining = await unreviewed();
  if (remaining >= total) throw new Error("nothing was acknowledged");
  if (remaining === 0) throw new Error("it acknowledged changes that were not shown");

  // What it did acknowledge is still on record.
  await page.getByPlaceholder("Search changes").fill("");
  await page.getByLabel("Filter changes", { exact: true }).selectOption("acknowledged");
  await page.waitForTimeout(300);
  if ((await page.locator("main ul > li").count()) === 0) {
    throw new Error("acknowledging deleted the records");
  }
  await page.getByLabel("Filter changes", { exact: true }).selectOption("unreviewed");
  return `${total} to ${remaining} unreviewed, ${groups} groups acknowledged`;
});

await step("Review opens the device with the change alongside it", async () => {
  await page.getByLabel("Filter changes", { exact: true }).selectOption("all");
  await page.waitForTimeout(250);
  await page.locator("main ul > li").first().getByRole("button", { name: "Review" }).click();
  const panel = page.getByRole("complementary");
  await panel.waitFor({ timeout: 3000 });
  if (!(await panel.innerText()).toLowerCase().includes("recorded changes")) {
    throw new Error("the drawer does not show the change");
  }
  await page.keyboard.press("Escape");
  return "drawer opened from Changes";
});

await step("Open the scan reaches the full comparison", async () => {
  await nav("Changes").click();
  await page.waitForTimeout(250);
  await page.locator("main ul > li").first().getByRole("button", { name: "Open the scan" }).click();
  await page.waitForTimeout(800);
  const text = await mainText();
  if (!/changes? since the previous scan|everything matched/.test(text)) {
    throw new Error(`the comparison did not open: ${text.slice(0, 160)}`);
  }
  for (const group of ["added devices", "missing devices", "changed devices"]) {
    if (!text.includes(group)) throw new Error(`comparison missing group: ${group}`);
  }
  return "scan-to-scan comparison intact";
});

// ---------------------------------------------------------------------------
// 7. Network naming
// ---------------------------------------------------------------------------

await step("settings names each network separately and the name propagates", async () => {
  await page.getByRole("button", { name: "Settings" }).click();
  const panel = page.getByRole("complementary", { name: "Settings" });
  await panel.waitFor({ timeout: 3000 });
  const fields = panel.getByLabel(/^Name for /);
  await fields.first().waitFor({ timeout: 3000 });
  const count = await fields.count();
  if (count < 2) throw new Error(`expected at least 2 networks, found ${count}`);
  const other = await fields.nth(1).inputValue();

  await fields.first().fill("Workshop");
  await fields.first().blur();
  await page
    .getByRole("status")
    .filter({ hasText: /renamed/i })
    .first()
    .waitFor({ timeout: 4000 });
  if ((await fields.nth(1).inputValue()) !== other) {
    throw new Error("renaming one network changed another");
  }
  await page.keyboard.press("Escape");
  await page.waitForTimeout(400);

  for (const view of ["Inventory", "Changes", "History"]) {
    await nav(view).click();
    await page.waitForTimeout(400);
    if (!/workshop/i.test(await mainText())) {
      throw new Error(`the new network name did not reach ${view}`);
    }
  }
  return `${count} networks, rename propagated everywhere`;
});

// ---------------------------------------------------------------------------
// 8. Scanning, streaming and partial-scan safety
// ---------------------------------------------------------------------------

await step("scan streams devices while running", async () => {
  await nav("Scan").click();
  await page.locator("#scan-target").fill("192.168.1.0/24");
  await scanButton().click();
  await page.locator("tbody tr").first().waitFor({ timeout: 5000 });
  await page.waitForTimeout(700);
  const midCount = await page.locator("tbody tr").count();
  const status = await page.locator("footer").first().innerText();
  if (midCount === 0) throw new Error("no rows appeared during the scan");
  if (!/devices found/.test(status)) throw new Error(`status bar did not show progress: ${status}`);
  return `${midCount} rows mid-scan`;
});

await step("a completed scan refreshes the Inventory", async () => {
  await page.getByRole("button", { name: "Stop" }).waitFor({ state: "detached", timeout: 30000 });
  await page.waitForTimeout(1000);
  const scanned = await page.locator("tbody tr").count();
  if (scanned === 0) throw new Error("the completed scan showed no devices");

  await nav("Inventory").click();
  await page.waitForTimeout(500);
  const headline = (await mainText()).match(/(\d+) devices · (\d+) present/);
  if (!headline) throw new Error("no summary after the scan");
  if (Number(headline[2]) < scanned) {
    throw new Error(`${headline[2]} present after a scan that found ${scanned}`);
  }
  return headline[0];
});

await step("Stop keeps partial results, marks nothing missing and adds no changes", async () => {
  await nav("Changes").click();
  await page.getByLabel("Filter changes", { exact: true }).selectOption("all");
  await page.waitForTimeout(400);
  const changesBefore = await page.locator("main ul > li").count();

  await nav("Inventory").click();
  await page.waitForTimeout(400);
  const before = (await mainText()).match(/(\d+) devices · (\d+) present · (\d+) missing/);

  await nav("Scan").click();
  await scanButton().click();
  await page.locator("tbody tr").first().waitFor({ timeout: 6000 });
  await page.waitForTimeout(400);
  const stop = page.getByRole("button", { name: /^Stop/ });
  await stop.click();
  await stop.waitFor({ state: "detached", timeout: 15000 });
  await page.waitForTimeout(1000);

  const kept = await page.locator("tbody tr").count();
  if (kept === 0) throw new Error("a stopped scan discarded the hosts it had found");

  await nav("Inventory").click();
  await page.waitForTimeout(500);
  const after = (await mainText()).match(/(\d+) devices · (\d+) present · (\d+) missing/);
  if (!after || !before) throw new Error("could not read the summary either side of the stop");
  if (after[3] !== before[3]) {
    throw new Error(`a partial scan changed the missing count from ${before[3]} to ${after[3]}`);
  }

  await nav("Changes").click();
  await page.waitForTimeout(400);
  const changesAfter = await page.locator("main ul > li").count();
  if (changesAfter !== changesBefore) {
    throw new Error(`a partial scan created ${changesAfter - changesBefore} change entries`);
  }
  return `${kept} devices kept, ${before[3]} missing unchanged, no new entries`;
});

await step("history labels the partial scan and refuses to compare it", async () => {
  await nav("History").click();
  await page.waitForTimeout(400);
  const newest = page.locator("main ul > li").first();
  const label = await newest.innerText();
  if (!/partial scan/i.test(label)) throw new Error(`newest scan not labelled partial: ${label}`);
  if (/\d+\s+missing/i.test(label)) {
    throw new Error(`a partial scan reported missing devices: ${label}`);
  }
  await newest.hover();
  const compare = newest.getByRole("button", { name: /stopped early|Compare/ });
  if (!(await compare.isDisabled())) throw new Error("a partial scan offered a comparison");
  return (await compare.getAttribute("aria-label")) ?? "";
});

await step("deleting a scan asks first and keeps the inventory", async () => {
  await nav("Inventory").click();
  await page.waitForTimeout(400);
  const devices = (await mainText()).match(/(\d+) devices/)?.[1];

  await nav("History").click();
  await page.waitForTimeout(300);
  await page.locator("main ul > li").first().hover();
  await page.getByRole("button", { name: "Delete this scan" }).first().click();
  await page.getByRole("dialog", { name: "Delete this scan?" }).waitFor({ timeout: 2000 });
  await page.getByRole("button", { name: "Delete scan" }).click();
  await page.waitForTimeout(800);

  await nav("Inventory").click();
  await page.waitForTimeout(500);
  const after = (await mainText()).match(/(\d+) devices/)?.[1];
  if (after !== devices) {
    throw new Error(`deleting a scan changed the inventory from ${devices} to ${after}`);
  }
  return `${devices} devices survived`;
});

// ---------------------------------------------------------------------------
// 9. Layout, themes and accessibility
// ---------------------------------------------------------------------------

await step("settings keeps the public IP lookup off by default", async () => {
  await page.getByRole("button", { name: "Settings" }).click();
  const panel = page.getByRole("complementary", { name: "Settings" });
  await panel.waitFor({ timeout: 3000 });
  const checked = await panel.locator("#settings-public-ip").getAttribute("aria-checked");
  if (checked !== "false") throw new Error(`public IP lookup should default off, got ${checked}`);
  if (await panel.getByRole("button", { name: "Check public IP" }).count()) {
    throw new Error("Check public IP offered while the lookup is disabled");
  }
  await page.keyboard.press("Escape");
  return "opt-in confirmed";
});

for (const width of [1440, 1280, 1024, 940]) {
  await step(`no horizontal overflow at ${width}px`, async () => {
    await page.setViewportSize({ width, height: 800 });
    await page.waitForTimeout(300);
    const problems = [];
    for (const view of ["Scan", "Inventory", "Changes", "History"]) {
      await nav(view).click();
      await page.waitForTimeout(250);
      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
      );
      if (overflow > 0) problems.push(`${view} overflows by ${overflow}px`);
    }
    if (problems.length) throw new Error(problems.join("; "));
  });
}

await step("the narrow layout keeps Device, Address, Status and Last seen", async () => {
  await page.setViewportSize({ width: 940, height: 700 });
  await nav("Inventory").click();
  await page.waitForTimeout(350);
  const headers = (await page.locator("thead th").allInnerTexts()).map((h) =>
    h.trim().toLowerCase(),
  );
  for (const needed of ["device", "address", "status", "last seen"]) {
    if (!headers.some((h) => h.includes(needed))) {
      throw new Error(`${needed} was hidden at 940px: ${headers.join(", ")}`);
    }
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  return `columns at 940px: ${headers.filter(Boolean).join(", ")}`;
});

await step("axe-core finds no violations across every view, in both themes", async () => {
  let AxeBuilder;
  try {
    AxeBuilder = (await import("@axe-core/playwright")).default;
  } catch {
    throw new Error(
      "@axe-core/playwright is not installed; run npm i --no-save @axe-core/playwright",
    );
  }
  const summary = [];
  const analyse = () =>
    new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
      .analyze();

  for (const theme of ["dark", "light"]) {
    await page.evaluate((value) => {
      const raw = localStorage.getItem("arcscan-settings");
      const settings = raw ? JSON.parse(raw) : {};
      settings.theme = value;
      localStorage.setItem("arcscan-settings", JSON.stringify(settings));
      localStorage.setItem("arcscan-theme", value);
    }, theme);
    await page.reload({ waitUntil: "networkidle" });
    await page.waitForTimeout(500);

    for (const view of ["Inventory", "Changes", "History"]) {
      await nav(view).click();
      await page.waitForTimeout(400);
      const result = await analyse();
      if (result.violations.length > 0) {
        throw new Error(
          result.violations
            .map((v) => `${theme}/${view} ${v.id} (${v.nodes.length}): ${v.help}`)
            .join("; "),
        );
      }
      summary.push(`${theme}/${view}: ${result.passes.length}`);
    }

    // A populated drawer with an active selection behind it, which a static
    // sweep of the table would miss.
    await nav("Inventory").click();
    await page.waitForTimeout(300);
    await page.locator('tbody input[type="checkbox"]').first().check();
    await page.locator("tbody tr").first().dblclick();
    await page.getByRole("complementary").waitFor({ timeout: 3000 });
    await page.waitForTimeout(400);
    const drawer = await analyse();
    if (drawer.violations.length > 0) {
      throw new Error(
        drawer.violations
          .map((v) => `${theme}/drawer ${v.id} (${v.nodes.length}): ${v.help}`)
          .join("; "),
      );
    }
    summary.push(`${theme}/drawer: ${drawer.passes.length}`);
    await page.keyboard.press("Escape");
    await page.keyboard.press("Escape");

    // The filtered-to-nothing state, which has its own heading and action.
    await page.getByPlaceholder("Search inventory").fill("zzzzz-no-such-device");
    await page.waitForTimeout(350);
    const empty = await analyse();
    if (empty.violations.length > 0) {
      throw new Error(
        empty.violations
          .map((v) => `${theme}/empty ${v.id} (${v.nodes.length}): ${v.help}`)
          .join("; "),
      );
    }
    summary.push(`${theme}/no-matches: ${empty.passes.length}`);
    await page.getByPlaceholder("Search inventory").fill("");
  }
  return summary.join(", ");
});

await step("no duplicate element ids anywhere in the tree", async () => {
  const duplicates = await page.evaluate(() => {
    const seen = new Map();
    for (const el of document.querySelectorAll("[id]")) {
      seen.set(el.id, (seen.get(el.id) ?? 0) + 1);
    }
    return [...seen.entries()].filter(([, n]) => n > 1).map(([id]) => id);
  });
  if (duplicates.length) throw new Error(`duplicate ids: ${duplicates.join(", ")}`);
});

if (errors.length > 0) {
  console.log(`\nFAIL  console clean — ${errors.length} error(s):`);
  for (const e of errors.slice(0, 8)) console.log(`      ${e}`);
  process.exitCode = 1;
} else {
  console.log("PASS  console clean — no errors or unhandled rejections");
}

await browser.close();
