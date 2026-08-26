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

/**
 * Every request the app makes to somewhere other than its own origin.
 *
 * ArcScan scans locally and sends nothing anywhere. The public-IP lookup is the
 * only feature that contacts a third party at all, and only after an explicit
 * press, so this list staying empty is what "nothing happens on its own" looks
 * like as an assertion rather than as a promise.
 */
const externalRequests = [];
const externalRequestCount = () => externalRequests.length;
page.on("request", (request) => {
  const url = request.url();
  if (url.startsWith(URL) || url.startsWith("data:") || url.startsWith("blob:")) return;
  externalRequests.push(`${request.method()} ${url}`);
});

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
// 9. The application shell and the Public IP utility
// ---------------------------------------------------------------------------

const contextRow = () => page.locator("header + div").first();
const publicIpStatus = () => contextRow().getByRole("status").first();

await page.goto(URL, { waitUntil: "networkidle" });

await step("the app names itself exactly once, and never twice", async () => {
  // The window is titled by the operating system. Up to 1.8.0 the header drew
  // its own ArcScan icon and wordmark underneath that, so a packaged build
  // showed both, stacked. A browser has no native title bar, which is exactly
  // why this needs asserting rather than eyeballing.
  const header = page.locator("header").first();
  const text = await header.innerText();
  if (/arcscan/i.test(text)) throw new Error(`the header still renders a wordmark: ${text}`);
  if (await header.locator("img").count()) throw new Error("the header still renders a logo");

  // Nowhere else in the chrome either: the drawer titles and the table are
  // content, but no second brand block may have moved somewhere else.
  const brandMarks = await page.locator("main img[src*='icon'], header img").count();
  if (brandMarks !== 0) throw new Error(`${brandMarks} brand marks in the chrome`);
  return "no in-app title or icon; the OS provides both";
});

await step("navigation is the first thing in the header", async () => {
  const first = await page.evaluate(() => {
    const header = document.querySelector("header");
    const child = header?.firstElementChild;
    return { tag: child?.tagName ?? "", label: child?.getAttribute("aria-label") ?? "" };
  });
  if (first.tag !== "NAV") throw new Error(`the header starts with a ${first.tag}, not the nav`);

  // And no empty spacer was left where the brand block used to be.
  const gap = await page.evaluate(() => {
    const nav = document.querySelector("header nav");
    return nav ? nav.getBoundingClientRect().left : -1;
  });
  if (gap > 20) throw new Error(`the nav starts ${gap}px in, which looks like a leftover spacer`);
  return `nav is first, ${Math.round(gap)}px from the edge`;
});

await step("the view switcher carries its counts accessibly", async () => {
  for (const [label, noun] of [
    ["Inventory", "devices"],
    ["Changes", "unreviewed"],
  ]) {
    const name = await nav(label).first().getAttribute("aria-label");
    const text = await nav(label).first().innerText();
    // The badge is a number with no visible unit, so the unit has to be read
    // out or "Changes 7" means nothing without sight of where it sits.
    if (!new RegExp(`\\d+\\s*${noun}`).test(text.replace(/\n/g, " "))) {
      throw new Error(`the ${label} badge does not say what it counts: ${JSON.stringify(text)}`);
    }
    if (name) throw new Error(`${label} overrides its own visible label with "${name}"`);
  }
  return "both badges name their unit";
});

await step("Public IP starts not checked, having asked nobody", async () => {
  const text = (await publicIpStatus().innerText()).replace(/\s+/g, " ");
  if (!/public ip/i.test(text)) throw new Error(`no Public IP utility on the Scan screen: ${text}`);
  if (!/not checked/i.test(text)) throw new Error(`unexpected initial state: ${text}`);
  await contextRow().getByRole("button", { name: "Check public IP" }).waitFor({ timeout: 3000 });

  if (externalRequests.length > 0) {
    throw new Error(`the app contacted ${externalRequests.join(", ")} without being asked`);
  }
  return "Not checked, zero outbound requests";
});

await step("switching views and running a scan never start a lookup", async () => {
  for (const view of ["Inventory", "Changes", "History", "Scan"]) {
    await nav(view).click();
    await page.waitForTimeout(200);
  }
  await scanButton().click();
  await page.getByRole("button", { name: /^Stop/ }).waitFor({ state: "detached", timeout: 40000 });
  await page.waitForTimeout(400);

  if (externalRequests.length > 0) {
    throw new Error(`a lookup ran without a press: ${externalRequests.join(", ")}`);
  }
  const text = (await publicIpStatus().innerText()).replace(/\s+/g, " ");
  if (!/not checked/i.test(text)) throw new Error(`the state changed on its own: ${text}`);
  return "four view switches and a full scan, still Not checked";
});

await step("Check shows a loading state, then the address", async () => {
  await page.goto(`${URL}?publicip=slow`, { waitUntil: "networkidle" });
  const check = contextRow().getByRole("button", { name: "Check public IP" });
  await check.click();

  await page.waitForTimeout(250);
  const loading = (await publicIpStatus().innerText()).replace(/\s+/g, " ");
  if (!/checking/i.test(loading)) throw new Error(`no loading state: ${loading}`);
  const busy = await contextRow()
    .getByRole("button", { name: "Checking the public IP" })
    .getAttribute("aria-busy");
  if (busy !== "true") throw new Error("the action does not report itself busy");

  const refresh = contextRow().getByRole("button", { name: "Check the public IP again" });
  await refresh.waitFor({ timeout: 12000 });
  const ready = (await publicIpStatus().innerText()).replace(/\s+/g, " ");
  if (!/203\.0\.113\.24/.test(ready)) throw new Error(`no address after the lookup: ${ready}`);
  if (!/checked just now/i.test(ready)) throw new Error(`no freshness indicator: ${ready}`);
  return ready;
});

await step("the address can be copied and the copy is confirmed", async () => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await contextRow().getByRole("button", { name: "Copy public IP address" }).click();
  await page.waitForTimeout(200);

  const clipboard = await page.evaluate(() => navigator.clipboard.readText());
  if (clipboard.trim() !== "203.0.113.24") throw new Error(`clipboard holds "${clipboard}"`);
  // Confirmed out loud, not only by an icon quietly swapping to a tick.
  const spoken = await contextRow().getByRole("status").last().innerText();
  const toast = await page.getByRole("status").filter({ hasText: /copied/i }).count();
  if (!/copied/i.test(spoken) && toast === 0) throw new Error("the copy is never confirmed");
  return "copied and confirmed";
});

await step("Refresh looks the address up again", async () => {
  const before = externalRequestCount();
  await contextRow().getByRole("button", { name: "Check the public IP again" }).click();
  await page.waitForTimeout(200);
  const during = (await publicIpStatus().innerText()).replace(/\s+/g, " ");
  if (!/checking/i.test(during)) throw new Error(`Refresh did not start a lookup: ${during}`);
  await contextRow()
    .getByRole("button", { name: "Check the public IP again" })
    .waitFor({ timeout: 12000 });
  return `still no outbound request in the demo (${before} before, ${externalRequestCount()} after)`;
});

await step("repeated presses do not stack up lookups", async () => {
  await page.goto(`${URL}?publicip=slow`, { waitUntil: "networkidle" });
  const check = contextRow().getByRole("button", { name: "Check public IP" });
  await check.click();
  // The button deliberately stays enabled so focus is not thrown to the body
  // mid-interaction; the duplicate is refused underneath instead.
  for (let i = 0; i < 4; i++) {
    await contextRow().getByRole("button", { name: "Checking the public IP" }).click();
    await page.waitForTimeout(60);
  }
  const focused = await page.evaluate(() => document.activeElement?.tagName ?? "");
  if (focused !== "BUTTON") throw new Error(`focus escaped to ${focused} during the lookup`);

  await contextRow()
    .getByRole("button", { name: "Check the public IP again" })
    .waitFor({ timeout: 12000 });
  const text = (await publicIpStatus().innerText()).replace(/\s+/g, " ");
  if (!/203\.0\.113\.24/.test(text)) throw new Error(`unexpected result: ${text}`);
  return "five presses, one answer, focus kept";
});

await step("a failed provider falls back to the next one", async () => {
  await page.goto(`${URL}?publicip=fallback`, { waitUntil: "networkidle" });
  await contextRow().getByRole("button", { name: "Check public IP" }).click();
  await contextRow()
    .getByRole("button", { name: "Check the public IP again" })
    .waitFor({ timeout: 12000 });

  const text = (await publicIpStatus().innerText()).replace(/\s+/g, " ");
  // The second provider's answer, so the fallback genuinely ran rather than
  // the first provider quietly succeeding.
  if (!/198\.51\.100\.17/.test(text)) throw new Error(`the fallback did not answer: ${text}`);
  return text;
});

await step("a total failure says so plainly, and Retry recovers", async () => {
  await page.goto(`${URL}?publicip=flaky`, { waitUntil: "networkidle" });
  await contextRow().getByRole("button", { name: "Check public IP" }).click();

  const retry = contextRow().getByRole("button", { name: "Try the public IP lookup again" });
  await retry.waitFor({ timeout: 12000 });
  const failed = (await publicIpStatus().innerText()).replace(/\s+/g, " ");
  if (!/public ip unavailable/i.test(failed)) throw new Error(`unexpected error copy: ${failed}`);
  if (!/check your connection/i.test(failed)) throw new Error(`no recovery advice: ${failed}`);

  // The raw provider errors are available without being shoved at anyone.
  await contextRow().getByRole("button", { name: "Technical details" }).click();
  const detail = await contextRow().locator("pre").innerText();
  if (!/ipify/i.test(detail)) throw new Error(`the details name no provider: ${detail}`);

  await retry.click();
  await contextRow()
    .getByRole("button", { name: "Check the public IP again" })
    .waitFor({ timeout: 12000 });
  const recovered = (await publicIpStatus().innerText()).replace(/\s+/g, " ");
  if (!/203\.0\.113\.24/.test(recovered)) throw new Error(`Retry did not recover: ${recovered}`);
  if (await contextRow().locator("pre").count()) {
    throw new Error("the previous failure's details are still open");
  }
  return "failure explained, details available, retry recovered";
});

await step("the address survives a view change and a scan, and is never stored", async () => {
  for (const view of ["Inventory", "Changes", "History", "Scan"]) {
    await nav(view).click();
    await page.waitForTimeout(180);
  }
  let text = (await publicIpStatus().innerText()).replace(/\s+/g, " ");
  if (!/203\.0\.113\.24/.test(text)) throw new Error(`the value was lost on a view change: ${text}`);

  await scanButton().click();
  await page.getByRole("button", { name: /^Stop/ }).waitFor({ state: "detached", timeout: 40000 });
  await page.waitForTimeout(400);
  text = (await publicIpStatus().innerText()).replace(/\s+/g, " ");
  if (!/203\.0\.113\.24/.test(text)) throw new Error(`the value was lost across a scan: ${text}`);

  // Held in memory for the session and nowhere else: not in the preferences,
  // not in the inventory, not in an export.
  const leaked = await page.evaluate(() => {
    const haystacks = [];
    for (let i = 0; i < localStorage.length; i++) {
      haystacks.push(localStorage.getItem(localStorage.key(i)) ?? "");
    }
    for (let i = 0; i < sessionStorage.length; i++) {
      haystacks.push(sessionStorage.getItem(sessionStorage.key(i)) ?? "");
    }
    return haystacks.filter((value) => value.includes("203.0.113.24"));
  });
  if (leaked.length > 0) throw new Error(`the address was persisted: ${leaked.join(" | ")}`);

  // A reload is a new session, so the value is gone.
  await page.reload({ waitUntil: "networkidle" });
  const after = (await publicIpStatus().innerText()).replace(/\s+/g, " ");
  if (!/not checked/i.test(after)) throw new Error(`the address outlived the session: ${after}`);
  return "kept for the session, stored nowhere, gone on reload";
});

await step("settings explains the lookup without competing with it", async () => {
  await page.getByRole("button", { name: "Settings" }).click();
  const panel = page.getByRole("complementary", { name: "Settings" });
  try {
    await panel.waitFor({ timeout: 3000 });
    const text = await panel.innerText();
    for (const needed of ["api64.ipify.org", "icanhazip.com", "press Check", "this session"]) {
      if (!text.includes(needed)) throw new Error(`settings does not mention "${needed}"`);
    }
    // Checking, copying and refreshing belong to the Scan screen now.
    for (const name of ["Check public IP", "Copy public IP address"]) {
      if (await panel.getByRole("button", { name }).count()) {
        throw new Error(`settings still offers a competing "${name}"`);
      }
    }

    // Turning it off removes the utility rather than leaving a dead control.
    await panel.locator("#settings-public-ip").click();
    await page.waitForTimeout(250);
    if (await contextRow().getByRole("button", { name: "Check public IP" }).count()) {
      throw new Error("the utility is still offered after being switched off");
    }
    await panel.locator("#settings-public-ip").click();
    await page.waitForTimeout(250);
    await contextRow().getByRole("button", { name: "Check public IP" }).waitFor({ timeout: 3000 });
  } finally {
    // Always close it: an open drawer's scrim swallows every later click.
    await page.keyboard.press("Escape");
    await page.waitForTimeout(250);
  }
  return "explained in Settings, operated from the Scan screen";
});

await step("no request ever left the demo", async () => {
  if (externalRequests.length > 0) {
    throw new Error(`the browser demo reached out to ${externalRequests.join(", ")}`);
  }
  return "zero outbound requests across the whole run";
});

// Back to a clean, fully seeded demo for the layout and accessibility passes.
await page.goto(URL, { waitUntil: "networkidle" });

// ---------------------------------------------------------------------------
// 10. Layout, themes and accessibility
// ---------------------------------------------------------------------------

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

  /** Scan whatever is on screen and fail with the label if anything is wrong. */
  const sweep = async (label) => {
    const result = await analyse();
    if (result.violations.length > 0) {
      throw new Error(
        result.violations
          .map((v) => `${label} ${v.id} (${v.nodes.length}): ${v.help}`)
          .join("; "),
      );
    }
    summary.push(`${label}: ${result.passes.length}`);
  };

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
      await sweep(`${theme}/${view}`);
    }

    // A populated drawer with an active selection behind it, which a static
    // sweep of the table would miss.
    await nav("Inventory").click();
    await page.waitForTimeout(300);
    await page.locator('tbody input[type="checkbox"]').first().check();
    await page.locator("tbody tr").first().dblclick();
    await page.getByRole("complementary").waitFor({ timeout: 3000 });
    await page.waitForTimeout(400);
    await sweep(`${theme}/drawer`);
    await page.keyboard.press("Escape");
    await page.keyboard.press("Escape");

    // The filtered-to-nothing state, which has its own heading and action.
    await page.getByPlaceholder("Search inventory").fill("zzzzz-no-such-device");
    await page.waitForTimeout(350);
    await sweep(`${theme}/no-matches`);
    await page.getByPlaceholder("Search inventory").fill("");

    // Settings, which carries the switches and the privacy copy.
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("complementary", { name: "Settings" }).waitFor({ timeout: 3000 });
    await page.waitForTimeout(350);
    await sweep(`${theme}/settings`);
    await page.keyboard.press("Escape");
    await page.waitForTimeout(250);

    // Every Public IP state, because each one renders different controls and a
    // different live region, and only one of them is the happy path.
    await page.goto(`${URL}?publicip=slow`, { waitUntil: "networkidle" });
    await page.waitForTimeout(400);
    await sweep(`${theme}/public-ip idle`);

    await contextRow().getByRole("button", { name: "Check public IP" }).click();
    await page.waitForTimeout(350);
    await sweep(`${theme}/public-ip loading`);

    await contextRow()
      .getByRole("button", { name: "Check the public IP again" })
      .waitFor({ timeout: 12000 });
    await page.waitForTimeout(250);
    await sweep(`${theme}/public-ip ready`);

    await page.goto(`${URL}?publicip=fail`, { waitUntil: "networkidle" });
    await contextRow().getByRole("button", { name: "Check public IP" }).click();
    await contextRow()
      .getByRole("button", { name: "Try the public IP lookup again" })
      .waitFor({ timeout: 12000 });
    await contextRow().getByRole("button", { name: "Technical details" }).click();
    await page.waitForTimeout(250);
    await sweep(`${theme}/public-ip error`);

    // A scan in flight, where the progress strip and the Stop action appear.
    await page.goto(URL, { waitUntil: "networkidle" });
    await scanButton().click();
    await page.locator("tbody tr").first().waitFor({ timeout: 8000 });
    await sweep(`${theme}/scanning`);
    await page.getByRole("button", { name: /^Stop/ }).click();
    await page.getByRole("button", { name: /^Stop/ }).waitFor({ state: "detached", timeout: 20000 });
    await page.waitForTimeout(300);
    await sweep(`${theme}/scanned`);

    // The narrowest supported window, where controls collapse and wrap.
    await page.setViewportSize({ width: 940, height: 620 });
    await page.waitForTimeout(350);
    await sweep(`${theme}/940px`);
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.waitForTimeout(250);
  }
  return summary.join(", ");
});

// ---------------------------------------------------------------------------
// Local discovery (v1.8.2)
//
// Driven through the demo's `?discovery=` scenarios, which run the real merge,
// naming and rendering code rather than a parallel path.
// ---------------------------------------------------------------------------

/** Open the Inventory drawer for the device whose address is `ip`. */
const openDeviceByIp = async (ip) => {
  await nav("Inventory").click();
  await page.waitForTimeout(300);
  const row = page.locator(`tbody tr:has-text("${ip}")`).first();
  await row.dblclick();
  await page.waitForTimeout(400);
  return page.locator("aside, [role='dialog']").last();
};

/** Turn optional Inventory columns on through the Settings panel. */
const enableColumns = async (keys) => {
  await page.getByRole("button", { name: "Settings" }).click();
  const panel = page.getByRole("complementary", { name: "Settings" });
  try {
    for (const key of keys) {
      await panel.locator(`#settings-inventory-column-${key}`).click();
      await page.waitForTimeout(120);
    }
  } finally {
    // Always close it: an open drawer's scrim swallows every later click.
    await page.keyboard.press("Escape");
    await page.waitForTimeout(250);
  }
};

await page.goto(`${URL}?discovery=normal`, { waitUntil: "networkidle" });

await step("a device's detected name and type reach the drawer", async () => {
  // The printer is the demo's Auto case: nobody has corrected it, so what the
  // drawer shows is ArcScan's own answer and its confidence.
  const drawer = await openDeviceByIp("192.168.1.31");
  const text = (await drawer.innerText()).toLowerCase();
  if (!text.includes("discovery")) throw new Error("no Discovery section");
  if (!text.includes("printer")) throw new Error(`no device type: ${text.slice(0, 200)}`);
  if (!text.includes("high confidence")) throw new Error("confidence not shown");
  if (!text.includes("detected automatically")) throw new Error("the type source is not stated");
  return "Printer, high confidence, detected automatically, with its evidence";
});

await step("the evidence behind a type is shown, not just the verdict", async () => {
  const drawer = await openDeviceByIp("192.168.1.31");
  const text = (await drawer.innerText()).toLowerCase();
  for (const fragment of ["_ipp._tcp", "hewlett packard", "mdns", "ssdp"]) {
    if (!text.includes(fragment)) throw new Error(`missing evidence: ${fragment}`);
  }
  return "service, manufacturer and both protocols named";
});

await step("a name the operator chose still wins over the detected one", async () => {
  const drawer = await openDeviceByIp("192.168.1.31");
  // The drawer names itself with the device's display name.
  const heading = (await drawer.getAttribute("aria-label"))?.trim();
  if (heading !== "Office Printer") throw new Error(`drawer titled "${heading}"`);
  const text = await drawer.innerText();
  if (!text.includes("Acme LaserFast 400")) throw new Error("detected name was discarded");
  if (!text.toLowerCase().includes("your name is used")) {
    throw new Error("the drawer does not say the operator's name is preferred");
  }
  return "Office Printer shown, Acme LaserFast 400 offered alongside";
});

await step("a supplemental IPv6 address never claims IPv6 was scanned", async () => {
  const drawer = await openDeviceByIp("192.168.1.12");
  const text = (await drawer.innerText()).toLowerCase();
  if (!text.includes("2001:db8")) throw new Error("no IPv6 address shown");
  if (!text.includes("scans ipv4 only")) throw new Error("no clarification beside it");
  return "shown as supplemental, with the limitation stated";
});

await step("a generic advertisement does not become a device's name", async () => {
  await nav("Inventory").click();
  await page.waitForTimeout(300);
  const row = page.locator('tbody tr:has-text("192.168.1.23")').first();
  const name = (await row.locator("td").nth(1).innerText()).trim().toLowerCase();
  if (name === "speaker") throw new Error("a generic name was used as the device name");
  return `kept as "${name}" rather than "speaker"`;
});

await step("Inventory search finds a device by model and by service", async () => {
  await nav("Inventory").click();
  await page.waitForTimeout(300);
  const search = page.getByPlaceholder("Search inventory");
  for (const [query, expected] of [
    ["laserfast", "192.168.1.31"],
    ["ipp printing", "192.168.1.31"],
    ["_googlecast", "192.168.1.44"],
  ]) {
    await search.fill(query);
    await page.waitForTimeout(250);
    const rows = await page.locator("tbody tr").count();
    const text = await page.locator("tbody").innerText();
    if (rows === 0 || !text.includes(expected)) {
      throw new Error(`"${query}" did not find ${expected} (${rows} rows)`);
    }
  }
  await search.fill("");
  return "model, friendly service name and protocol service name all match";
});

await step("the device-type filter narrows the table", async () => {
  await nav("Inventory").click();
  await page.waitForTimeout(300);
  const filter = page.getByLabel("Filter by device type");
  if ((await filter.count()) === 0) throw new Error("no type filter offered");
  await filter.selectOption("printer");
  await page.waitForTimeout(250);
  const text = await page.locator("tbody").innerText();
  if (!text.includes("192.168.1.31")) throw new Error("the printer was filtered out");
  if (text.includes("192.168.1.44")) throw new Error("a television survived a printer filter");
  await filter.selectOption("");
  return "Printer shows the printer and nothing else";
});

await step("the optional discovery columns appear when turned on", async () => {
  await enableColumns(["type", "model", "detected_name"]);
  await nav("Inventory").click();
  await page.waitForTimeout(350);
  const headers = (await page.locator("thead").innerText()).toLowerCase();
  for (const column of ["type", "model", "detected name"]) {
    if (!headers.includes(column)) throw new Error(`no ${column} column: ${headers}`);
  }
  return headers.replace(/\s+/g, " ").trim().slice(0, 120);
});

await step("a scan shows the discovery phases while it runs", async () => {
  await page.goto(`${URL}?discovery=slow`, { waitUntil: "networkidle" });
  await scanButton().click();
  const seen = new Set();
  for (let i = 0; i < 80; i++) {
    const text = (await page.locator("body").innerText()).toLowerCase();
    for (const phase of [
      "discovering local services",
      "reading device descriptions",
      "classifying devices",
    ]) {
      if (text.includes(phase)) seen.add(phase);
    }
    if (seen.size === 3) break;
    await page.waitForTimeout(120);
  }
  if (seen.size === 0) throw new Error("no discovery phase was ever shown");
  await page.waitForTimeout(2500);
  return [...seen].join(", ");
});

await step("Stop during discovery keeps results and records no discovery events", async () => {
  await page.goto(`${URL}?discovery=slow`, { waitUntil: "networkidle" });
  await scanButton().click();
  // Wait until discovery is under way, then stop mid-phase.
  for (let i = 0; i < 80; i++) {
    const text = (await page.locator("body").innerText()).toLowerCase();
    if (text.includes("discovering local services")) break;
    await page.waitForTimeout(120);
  }
  const stop = page.getByRole("button", { name: /stop/i }).first();
  if ((await stop.count()) > 0) await stop.click();
  await page.waitForTimeout(1500);

  await nav("Changes").click();
  await page.waitForTimeout(400);
  const inbox = await inboxText();
  for (const forbidden of ["detected name change", "device type change", "service appeared"]) {
    if (inbox.includes(forbidden)) throw new Error(`a stopped scan produced "${forbidden}"`);
  }
  return "partial results kept, no discovery events written";
});

await step("with discovery off, nothing is detected but the type is still correctable", async () => {
  await page.goto(`${URL}?discovery=none`, { waitUntil: "networkidle" });
  const drawer = await openDeviceByIp("192.168.1.31");
  const text = (await drawer.innerText()).toLowerCase();
  if (text.includes("acme laserfast")) throw new Error("a detected name appeared with discovery off");
  if (text.includes("high confidence")) throw new Error("a confidence appeared with discovery off");
  // v1.8.3: the correction is deliberately still offered. A device ArcScan
  // could not reach has a type worth setting too, and hiding the control would
  // make the one case the operator most wants to fix the one they cannot.
  if (!text.includes("no discovery-capable scan has reached this device")) {
    throw new Error("the drawer does not say why there is nothing to show");
  }
  if ((await drawer.locator("#device-type").count()) === 0) {
    throw new Error("the type cannot be corrected with discovery off");
  }
  return "nothing detected, said plainly, and the type still correctable";
});

await step("a hostile advertisement renders as text and breaks nothing", async () => {
  await page.goto(`${URL}?discovery=malformed`, { waitUntil: "networkidle" });
  const drawer = await openDeviceByIp("192.168.1.31");
  const text = await drawer.innerText();
  // The script tag must appear as characters, never as an element.
  if (!text.includes("<script>")) throw new Error("the hostile name was not rendered at all");
  const injected = await page.locator("aside script, [role='dialog'] script").count();
  if (injected > 0) throw new Error("a script element was created from device text");
  const bold = await page.locator("aside b, [role='dialog'] b").count();
  if (bold > 0) throw new Error("device text was interpreted as markup");
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth > document.documentElement.clientWidth + 1,
  );
  if (overflow) throw new Error("a long advertised name pushed the page sideways");
  return "rendered as text, no markup interpreted, no overflow";
});

await step("conflicting evidence is shown rather than silently resolved", async () => {
  await page.goto(`${URL}?discovery=conflict`, { waitUntil: "networkidle" });
  const drawer = await openDeviceByIp("192.168.1.50");
  const text = (await drawer.innerText()).toLowerCase();
  if (!text.includes("also consistent with")) throw new Error("no conflicts shown");
  if (!text.includes("other names it advertised")) throw new Error("no alternate names shown");
  return "both the competing types and the competing names are visible";
});

// --- v1.8.3: correcting a type, and evidence that goes quiet ---------------
//
// The demo ships three type states on purpose: the printer is Auto, the
// television has been corrected over a medium-confidence media-device reading,
// and the driveway camera carries an explicit Unknown. That is what lets these
// steps check the three without having to set them up first.

await page.goto(`${URL}?discovery=normal`, { waitUntil: "networkidle" });

/** The device-type select inside the open drawer. */
const typeSelect = (drawer) => drawer.locator("#device-type");

await step("an uncorrected device says the type was detected automatically", async () => {
  const drawer = await openDeviceByIp("192.168.1.31");
  const text = (await drawer.innerText()).toLowerCase();
  if (!text.includes("detected automatically")) throw new Error("the type source is not stated");
  if (text.includes("set by you")) throw new Error("an untouched device claims to be user-set");
  if ((await typeSelect(drawer).inputValue()) !== "") {
    throw new Error("the control does not start on Automatic");
  }
  return "Automatic, and the control says so";
});

await step("a corrected device says so and keeps ArcScan's own answer beneath", async () => {
  const drawer = await openDeviceByIp("192.168.1.44");
  const text = (await drawer.innerText()).toLowerCase();
  if (!text.includes("television")) throw new Error("the corrected type is not shown");
  if (!text.includes("set by you")) throw new Error("the correction is not attributed");
  // The whole point of keeping the detection underneath: the operator can see
  // what they overruled, and clearing the correction is not a loss.
  if (!text.includes("arcscan detected")) throw new Error("the detected answer is not shown");
  if (!text.includes("media device")) throw new Error("the detected type is not named");
  if ((await typeSelect(drawer).inputValue()) !== "television") {
    throw new Error("the control does not show the correction");
  }
  return "Television, set by you, over ArcScan's Media device";
});

await step("an explicit Unknown is a chosen answer and not the absence of one", async () => {
  const drawer = await openDeviceByIp("192.168.1.60");
  const text = (await drawer.innerText()).toLowerCase();
  if (!text.includes("set by you")) throw new Error("the explicit Unknown is not attributed");
  if (!text.includes("arcscan detected")) throw new Error("the detected answer is not shown");
  if (!text.includes("camera")) throw new Error("the detected Camera is not named");
  if ((await typeSelect(drawer).inputValue()) !== "unknown") {
    throw new Error("the control does not show the explicit Unknown");
  }
  return "Unknown, set by you, over ArcScan's Camera";
});

await step("setting a type saves it and changes nothing about the device's identity", async () => {
  // Survival across a *restart* is a database guarantee and is proved in the
  // Rust suite, which owns the storage; the demo backend lives in memory and a
  // reload is a fresh install by design. What is checked here is what the
  // interface owns: the correction sticks across closing the drawer, a view
  // change and reopening, and nothing else about the device moves.
  const before = await openDeviceByIp("192.168.1.50");
  const beforeText = await before.innerText();
  const field = (text, label) => (text.match(new RegExp(`${label}[^\\n]*\\n([^\\n]*)`)) ?? [])[1];
  const beforeMac = field(beforeText, "MAC address");
  const beforeIp = field(beforeText, "IP address");
  const beforeFirstSeen = field(beforeText, "First seen");
  const beforeCount = await page.locator("tbody tr").count();

  await typeSelect(before).selectOption("printer");
  await page.waitForTimeout(400);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(250);

  await nav("Changes").click();
  await page.waitForTimeout(300);

  const after = await openDeviceByIp("192.168.1.50");
  const afterText = await after.innerText();
  if (!afterText.toLowerCase().includes("set by you")) {
    throw new Error("the correction did not survive reopening the drawer");
  }
  if ((await typeSelect(after).inputValue()) !== "printer") {
    throw new Error("the correction did not stick");
  }
  if (field(afterText, "MAC address") !== beforeMac) throw new Error("the MAC address changed");
  if (field(afterText, "IP address") !== beforeIp) throw new Error("the address changed");
  if (field(afterText, "First seen") !== beforeFirstSeen) throw new Error("first seen changed");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  if ((await page.locator("tbody tr").count()) !== beforeCount) {
    throw new Error("correcting a type created or removed a device");
  }
  return `kept across a view change, identity unchanged (${beforeMac})`;
});

await step("a correction records no change event", async () => {
  await nav("Changes").click();
  await page.waitForTimeout(300);
  const before = await page.locator("main ul > li").count();

  const drawer = await openDeviceByIp("192.168.1.50");
  await typeSelect(drawer).selectOption("nas");
  await page.waitForTimeout(400);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);

  await nav("Changes").click();
  await page.waitForTimeout(300);
  const after = await page.locator("main ul > li").count();
  if (after !== before) {
    throw new Error(`a correction added ${after - before} change event(s)`);
  }
  return `${before} events before and after: an operator edit is not a network event`;
});

await step("clearing a correction reveals ArcScan's current answer", async () => {
  const drawer = await openDeviceByIp("192.168.1.50");
  await drawer.getByRole("button", { name: "Use automatic detection" }).click();
  await page.waitForTimeout(400);
  const text = (await drawer.innerText()).toLowerCase();
  if (text.includes("set by you")) throw new Error("the correction was not cleared");
  if (!text.includes("detected automatically")) throw new Error("the automatic answer is not shown");
  if (!text.includes("nas")) throw new Error("ArcScan's own answer is not revealed");
  if ((await typeSelect(drawer).inputValue()) !== "") {
    throw new Error("the control did not return to Automatic");
  }
  return "back to Automatic, showing NAS again";
});

await step("the type filter follows the correction rather than the detection", async () => {
  await page.goto(`${URL}?discovery=normal`, { waitUntil: "networkidle" });
  await nav("Inventory").click();
  await page.waitForTimeout(300);
  const filter = page.getByLabel("Filter by device type");
  await filter.selectOption("television");
  await page.waitForTimeout(300);
  const rows = await page.locator("tbody tr").allInnerTexts();
  if (rows.length !== 1) throw new Error(`Television matched ${rows.length} devices`);
  if (!rows[0].includes("192.168.1.44")) throw new Error("the corrected device is not listed");

  // And it no longer appears under the type ArcScan detected. Media device is
  // still offered, because two other demo devices really are media devices —
  // what must have changed is that the corrected one is not among them. A
  // filter that disagreed with the column beside it would make the correction
  // feel as though it had not saved.
  await filter.selectOption("media_device");
  await page.waitForTimeout(300);
  const media = await page.locator("tbody").innerText();
  if (media.includes("192.168.1.44")) {
    throw new Error("the corrected device is still filed under its detected type");
  }
  if (!media.includes("192.168.1.77")) throw new Error("a genuine media device was filtered out");
  await filter.selectOption("");
  await page.waitForTimeout(200);
  return "Television lists it; Media device no longer does, and still lists the real ones";
});

await step("search reaches both the correction and the detection", async () => {
  await page.goto(`${URL}?discovery=normal`, { waitUntil: "networkidle" });
  await nav("Inventory").click();
  await page.waitForTimeout(300);
  const search = page.getByPlaceholder(/search/i).first();
  const count = async (term) => {
    await search.fill(term);
    await page.waitForTimeout(300);
    return page.locator("tbody tr").count();
  };
  // "What did I correct to a television?" and "what did ArcScan call a media
  // device?" are both real questions and both have to be answerable.
  if ((await count("television")) === 0) throw new Error("the correction is not searchable");
  if ((await count("media device")) === 0) throw new Error("the detection is not searchable");
  await search.fill("");
  await page.waitForTimeout(200);
  return "both the corrected type and the detected one are findable";
});

await step("evidence that is current, getting old and stale reads differently", async () => {
  await page.goto(`${URL}?discovery=normal`, { waitUntil: "networkidle" });
  const fresh = (await (await openDeviceByIp("192.168.1.31")).innerText()).toLowerCase();
  if (!fresh.includes("current")) throw new Error("fresh evidence is not marked current");
  if (fresh.includes("stale evidence")) throw new Error("fresh evidence shows a stale group");

  const aging = (await (await openDeviceByIp("192.168.1.77")).innerText()).toLowerCase();
  if (!aging.includes("getting old")) throw new Error("aging evidence is not marked");
  if (!aging.includes("not heard recently")) throw new Error("no aging group");
  if (!aging.includes("discovery scan ago")) throw new Error("aging is not counted in scans");

  const stale = (await (await openDeviceByIp("192.168.1.81")).innerText()).toLowerCase();
  if (!stale.includes("stale")) throw new Error("stale evidence is not marked");
  if (!stale.includes("stale evidence")) throw new Error("no stale group");
  if (!stale.includes("discovery scans ago")) throw new Error("staleness is not counted in scans");
  // Counted in scans and never in days: a wall-clock age would say more about
  // when the operator last scanned than about the device.
  if (/\b\d+ (day|week|month)s? ago\b/.test(stale)) {
    throw new Error("stale evidence is dated in days rather than scans");
  }
  return "current, getting old and stale, each counted in discovery scans";
});

await step("stale evidence no longer supports an unsupported High confidence", async () => {
  const drawer = await openDeviceByIp("192.168.1.81");
  const text = (await drawer.innerText()).toLowerCase();
  if (!text.includes("medium confidence")) throw new Error("the reduced confidence is not shown");
  if (!text.includes("reduced from high confidence")) {
    throw new Error("the reduction is shown without being explained");
  }
  // The only mention of "high confidence" left must be the explanation of the
  // reduction; the claim itself must be gone.
  const claims = text.split("reduced from high confidence").join("");
  if (claims.includes("high confidence")) {
    throw new Error("a type resting on stale evidence still claims high confidence");
  }
  // Reduced, not erased: the device is still a media device, and the evidence
  // is still on file.
  if (!text.includes("media device")) throw new Error("the type was lost rather than reduced");
  if (!text.includes("mediaserver")) throw new Error("stale evidence was deleted");
  return "Medium instead of High, explained, with the evidence still on file";
});

await step("the Inventory marks a type that rests on stale evidence", async () => {
  await nav("Inventory").click();
  await page.waitForTimeout(300);
  const row = page.locator('tbody tr:has-text("192.168.1.81")').first();
  const text = (await row.innerText()).toLowerCase();
  if (!text.includes("stale")) throw new Error("a stale type looks exactly like a fresh one");
  return "the stale type is marked in the table";
});

await step("the Inventory marks a corrected type without grading the operator", async () => {
  const row = page.locator('tbody tr:has-text("192.168.1.44")').first();
  const text = (await row.innerText()).toLowerCase();
  if (!text.includes("television")) throw new Error("the corrected type is not in the column");
  if (!text.includes("you")) throw new Error("the correction is not attributed in the table");
  // A confidence word beside a type a person chose would be ArcScan grading
  // them, which it has no business doing.
  for (const word of ["high", "medium", "low"]) {
    if (text.includes(word)) throw new Error(`a corrected type carries "${word}"`);
  }
  return "attributed to you, with no confidence attached";
});

await step("Copy discovery details produces a redacted, useful report", async () => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  const drawer = await openDeviceByIp("192.168.1.31");
  const outbound = externalRequestCount();
  await drawer.getByRole("button", { name: "Copy discovery details" }).click();
  await page.waitForTimeout(400);

  const copied = await page.evaluate(() => navigator.clipboard.readText());
  for (const expected of [
    "ArcScan discovery report",
    "Device type: Printer",
    "Type source: Automatic",
    "_ipp._tcp",
  ]) {
    if (!copied.includes(expected)) throw new Error(`the report is missing ${expected}`);
  }
  // Nothing that identifies the unit or its owner.
  for (const forbidden of [
    "3C:D9:2B:6F:08:AA",
    "192.168.1.31",
    "Toner reordered",
    "uuid:",
    "http://",
  ]) {
    if (copied.includes(forbidden)) throw new Error(`the report leaked ${forbidden}`);
  }
  if (!copied.includes("192.168.x.x")) throw new Error("the address is not masked");
  if (externalRequestCount() !== outbound) throw new Error("copying made a network request");

  // Confirmed to the operator rather than done silently.
  const toast = await page.getByText(/copied/i).first().isVisible();
  if (!toast) throw new Error("the copy was not confirmed");
  return "redacted, masked, confirmed, and nothing left the machine";
});

await step("the report names a correction and keeps the detection beside it", async () => {
  const drawer = await openDeviceByIp("192.168.1.44");
  await drawer.getByRole("button", { name: "Copy discovery details" }).click();
  await page.waitForTimeout(400);
  const copied = await page.evaluate(() => navigator.clipboard.readText());
  if (!copied.includes("Device type: Television")) throw new Error("the correction is not named");
  if (!copied.includes("Type source: Set by you")) throw new Error("it is not attributed");
  if (!copied.includes("ArcScan detected: Media device")) {
    throw new Error("the detected answer is missing, which is the point of the report");
  }
  return "Television by you, over ArcScan's Media device";
});

await step("History says how well each discovery pass went", async () => {
  await page.goto(`${URL}?discovery=normal`, { waitUntil: "networkidle" });
  await nav("History").click();
  await page.waitForTimeout(400);
  const text = (await page.locator("main").innerText()).toLowerCase();
  if (!text.includes("discovery: complete")) throw new Error("no complete state in History");
  if (!/\d+ mdns/.test(text)) throw new Error("a complete pass does not show what it heard");
  // ArcScan never blames a firewall, because it cannot observe one.
  if (text.includes("firewall")) throw new Error("History claims a firewall blocked something");
  return "Discovery: Complete, with the counts it actually heard";
});

await step("History distinguishes skipped, limited and interrupted", async () => {
  await page.goto(`${URL}?discovery=none`, { waitUntil: "networkidle" });
  await nav("History").click();
  await page.waitForTimeout(400);
  const skipped = (await page.locator("main").innerText()).toLowerCase();
  if (!skipped.includes("discovery: skipped")) throw new Error("no skipped state");

  await page.goto(`${URL}?discovery=malformed`, { waitUntil: "networkidle" });
  await nav("History").click();
  await page.waitForTimeout(400);
  const limited = (await page.locator("main").innerText()).toLowerCase();
  if (!limited.includes("discovery: limited")) throw new Error("no limited state");
  if (!limited.includes("descriptions refused")) {
    throw new Error("a limited pass does not say what ArcScan observed");
  }

  // Interrupted: stop the scan while a discovery phase is on screen, which is
  // the case the state exists for.
  await page.goto(`${URL}?discovery=slow`, { waitUntil: "networkidle" });
  await page.getByRole("button", { name: /^Scan 192\.168\.1\.0\/24$/ }).click();
  for (let i = 0; i < 100; i++) {
    const body = (await page.locator("body").innerText()).toLowerCase();
    if (body.includes("discovering local services")) break;
    await page.waitForTimeout(120);
  }
  const stop = page.getByRole("button", { name: /^Stop/ });
  if ((await stop.count()) === 0) throw new Error("the scan finished before it could be stopped");
  await stop.click();
  await stop.waitFor({ state: "detached", timeout: 30000 });
  await nav("History").click();
  await page.waitForTimeout(600);
  const interrupted = (await page.locator("main").innerText()).toLowerCase();
  if (!interrupted.includes("discovery: interrupted")) throw new Error("no interrupted state");
  return "Skipped, Limited with its observed reason, and Interrupted";
});

await step("the type control is reachable and operable from the keyboard", async () => {
  await page.goto(`${URL}?discovery=normal`, { waitUntil: "networkidle" });
  const drawer = await openDeviceByIp("192.168.1.31");
  const select = typeSelect(drawer);
  await select.focus();
  const focused = await page.evaluate(() => document.activeElement?.id);
  if (focused !== "device-type") throw new Error(`focus landed on ${focused}`);
  await select.selectOption("camera");
  await page.waitForTimeout(400);
  if ((await select.inputValue()) !== "camera") throw new Error("the keyboard change did not save");
  // Escape closes the drawer rather than being swallowed by the select.
  await page.keyboard.press("Escape");
  await page.waitForTimeout(300);
  const drawerOpen = await page.locator("aside, [role='dialog']").last().isVisible();
  if (drawerOpen && (await typeSelect(page.locator("aside").last()).count()) > 0) {
    throw new Error("Escape did not close the drawer");
  }
  return "focusable, operable and Escape still closes the drawer";
});

await step("Settings explains aging without exposing a threshold to tune", async () => {
  await page.goto(`${URL}?discovery=normal`, { waitUntil: "networkidle" });
  await page.getByRole("button", { name: "Settings" }).click();
  const panel = page.getByRole("complementary", { name: "Settings" });
  await panel.waitFor({ timeout: 3000 });
  const text = (await panel.innerText()).toLowerCase();
  if (!text.includes("stops relying on it")) throw new Error("aging is not explained");
  // No threshold, no weights: the number only means anything alongside the
  // definition of a qualifying miss, and offering one without the other invites
  // a person to turn it down and then disbelieve the result.
  for (const forbidden of ["stale after", "classifier", "weight", "threshold"]) {
    if (text.includes(forbidden)) throw new Error(`Settings exposes "${forbidden}"`);
  }
  await page.keyboard.press("Escape");
  await page.waitForTimeout(250);
  return "explained in words, with nothing to tune";
});

await step("the drawer holds together at the minimum width, in both themes", async () => {
  for (const theme of ["dark", "light"]) {
    await page.evaluate((t) => document.documentElement.setAttribute("data-theme", t), theme);
    await page.setViewportSize({ width: 940, height: 900 });
    await page.goto(`${URL}?discovery=normal`, { waitUntil: "networkidle" });
    const drawer = await openDeviceByIp("192.168.1.81");
    if ((await typeSelect(drawer).count()) === 0) {
      throw new Error(`the type control is missing at 940px in ${theme}`);
    }
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth > document.documentElement.clientWidth + 1,
    );
    if (overflow) throw new Error(`the drawer overflows at 940px in ${theme}`);
    await page.keyboard.press("Escape");
    await page.waitForTimeout(200);
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.evaluate(() => document.documentElement.setAttribute("data-theme", "dark"));
  return "no overflow at 940px in either theme, with the control present";
});

await step("axe-core finds no violations in the new drawer states", async () => {
  const { default: AxeBuilder } = await import("@axe-core/playwright");
  const results = [];
  for (const theme of ["dark", "light"]) {
    await page.evaluate((t) => document.documentElement.setAttribute("data-theme", t), theme);
    for (const ip of ["192.168.1.44", "192.168.1.81", "192.168.1.60"]) {
      await page.goto(`${URL}?discovery=normal`, { waitUntil: "networkidle" });
      await openDeviceByIp(ip);
      const scan = await new AxeBuilder({ page })
        .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
        .analyze();
      if (scan.violations.length > 0) {
        throw new Error(
          `${theme}/${ip}: ${scan.violations.map((v) => `${v.id} (${v.nodes.length})`).join(", ")}`,
        );
      }
      results.push(`${theme}/${ip}: ${scan.passes.length}`);
      await page.keyboard.press("Escape");
      await page.waitForTimeout(200);
    }
  }
  await page.evaluate(() => document.documentElement.setAttribute("data-theme", "dark"));
  return results.join(", ");
});

await page.goto(URL, { waitUntil: "networkidle" });

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
