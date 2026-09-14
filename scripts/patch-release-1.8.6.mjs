import fs from "node:fs";
import { execFileSync } from "node:child_process";

const VERSION = "1.8.6";

function read(path) {
  return fs.readFileSync(path, "utf8");
}
function write(path, text) {
  fs.writeFileSync(path, text);
}
function replaceOnce(text, before, after, label) {
  const i = text.indexOf(before);
  if (i < 0) throw new Error(`${label}: expected text not found`);
  if (text.indexOf(before, i + before.length) >= 0) throw new Error(`${label}: matched more than once`);
  return text.slice(0, i) + after + text.slice(i + before.length);
}

// package.json is the release version source of truth.
const pkg = JSON.parse(read("package.json"));
if (pkg.version !== "1.8.5") throw new Error(`package.json expected 1.8.5, got ${pkg.version}`);
pkg.version = VERSION;
write("package.json", JSON.stringify(pkg, null, 2) + "\n");

// Keep npm's root package metadata aligned without touching dependency versions.
const lock = JSON.parse(read("package-lock.json"));
lock.version = VERSION;
if (!lock.packages?.[""]) throw new Error("package-lock root package missing");
lock.packages[""].version = VERSION;
write("package-lock.json", JSON.stringify(lock, null, 2) + "\n");

// Existing helper updates Cargo.toml, Tauri config, site fallbacks and privacy.
execFileSync(process.execPath, ["scripts/sync-version.mjs"], { stdio: "inherit" });

// Cargo.lock carries ArcScan's own package version separately.
let cargoLock = read("src-tauri/Cargo.lock");
cargoLock = replaceOnce(
  cargoLock,
  'name = "arcscan"\nversion = "1.8.5"',
  'name = "arcscan"\nversion = "1.8.6"',
  "Cargo.lock ArcScan package",
);
write("src-tauri/Cargo.lock", cargoLock);

// Changelog entry for the focused patch release.
let changelog = read("CHANGELOG.md");
const changelogMarker = "## [1.8.5] - 2026-09-14";
const changelogEntry = `## [1.8.6] - 2026-09-14\n\nA focused macOS LAN-discovery reliability and ArcAtlas UI polish release. Full notes:\n[docs/RELEASE-NOTES-1.8.6.md](docs/RELEASE-NOTES-1.8.6.md).\n\n### Fixed\n\n- **macOS ARP discovery no longer stalls on reverse DNS.** Local neighbor-table reads now use\n  numeric output with \`arp -n -a\`, preventing hostname resolution across a populated subnet\n  from consuming ArcScan's ARP-read timeout and leaving only a handful of direct responders.\n- **macOS ICMP probes stay numeric and use the platform reply timeout.** Ping now avoids name\n  lookups, waits only for the configured reply window, and exits after the first valid response.\n- The 1.8.5 positive ICMP/TCP retention, quiet-device ARP discovery and proxy-ARP filtering\n  remain in place.\n\n### Improved\n\n- **Simplified ArcAtlas controls.** Inventory now presents **Send to ArcAtlas** as the single\n  toolbar action instead of placing a redundant **ArcAtlas** button beside it. Connection\n  management remains available from the send flow.\n\n`;
if (!changelog.includes(changelogMarker)) throw new Error("CHANGELOG 1.8.5 marker missing");
changelog = changelog.replace(changelogMarker, changelogEntry + changelogMarker);
write("CHANGELOG.md", changelog);

// Refresh the homepage release presentation.
let home = read("site/index.html");
home = home.replace("<!-- ArcScan site build: v1.8.4 -->", "<!-- ArcScan site build: v1.8.6 -->");
home = replaceOnce(home, 'href="whats-new-1.8.5.html"\n              >What changed in 1.8.5</a', 'href="whats-new-1.8.6.html"\n              >What changed in 1.8.6</a', "hero release link");

const sectionStart = home.indexOf("      <!-- ====================================================== new in 1.8.5 -->");
const sectionEnd = home.indexOf('      <section class="section" id="download">', sectionStart);
if (sectionStart < 0 || sectionEnd < 0) throw new Error("homepage release section markers missing");
const newSection = `      <!-- ====================================================== new in 1.8.6 -->\n      <section class="section" id="whats-new">\n        <div class="wrap">\n          <div class="section-head">\n            <p class="eyebrow">New in 1.8.6</p>\n            <h2>More reliable local discovery on macOS.</h2>\n            <p>\n              ArcScan now keeps macOS ARP and ICMP discovery numeric so reverse-DNS lookups cannot\n              consume the scanner's short liveness timeouts on a populated LAN. This targets the\n              failure mode where Inventory remembers many devices but a fresh scan ends with only a few.\n              <a href="whats-new-1.8.6.html">What&rsquo;s new in 1.8.6</a>\n            </p>\n          </div>\n\n          <div class="whats-new">\n            <article>\n              <h3>ARP without reverse-DNS stalls</h3>\n              <p>\n                macOS neighbor-table reads now use numeric <code>arp -n -a</code> output. Hostname\n                lookups can no longer burn through the ARP-read budget and discard the local device table.\n              </p>\n            </article>\n\n            <article>\n              <h3>Numeric ICMP probes</h3>\n              <p>\n                macOS ping stays numeric, uses the platform's per-reply timeout, and exits after the\n                first valid response. Positive ICMP/TCP evidence and proxy-ARP protection remain intact.\n              </p>\n            </article>\n\n            <article>\n              <h3>One clear ArcAtlas action</h3>\n              <p>\n                Inventory now shows Send to ArcAtlas as the single ArcAtlas toolbar action. The redundant\n                neighboring ArcAtlas button is gone, while connection management remains in the send flow.\n              </p>\n            </article>\n          </div>\n        </div>\n      </section>\n\n`;
home = home.slice(0, sectionStart) + newSection + home.slice(sectionEnd);
write("site/index.html", home);

// Update only the verifier's current-release expectations. The historical 1.8.4
// deep regression suite intentionally remains pinned to 1.8.4.
let verify = read("scripts/verify-site.mjs");
verify = replaceOnce(
  verify,
  `await step("the release section states the 1.8.5 improvements", async () => {\n  const section = page.locator("#whats-new");\n  await section.waitFor({ timeout: 3000 });\n  const text = (await section.innerText()).toLowerCase();\n\n  if (!text.includes("1.8.5")) throw new Error("the section does not name the version");\n\n  const claims = [\n    [/arcatlas/, "the ArcAtlas handoff"],\n    [/one selected network|choose a network/, "one-network scope"],\n    [/explicit|deliberate send|confirm/, "explicit operator action"],\n    [/nothing is sent when a scan merely finishes|nothing is sent.*scan/, "no automatic post-scan upload"],\n    [/credential store/, "installed credential-store secret handling"],\n    [/process memory/, "portable in-memory secret handling"],\n    [/icmp|tcp/, "positive probe evidence"],\n    [/arp cache|proxy-arp/, "macOS ARP finalization protection"],\n  ];\n  for (const [pattern, label] of claims) {\n    if (!pattern.test(text)) throw new Error(\`the section does not cover \${label}\`);\n  }\n\n  const headings = await section.locator("h3").allInnerTexts();\n  if (headings.length !== 3) throw new Error(\`expected 3 improvements, got \${headings.length}\`);\n  return headings.map((h) => h.trim()).join(", ");\n});`,
  `await step("the release section states the 1.8.6 improvements", async () => {\n  const section = page.locator("#whats-new");\n  await section.waitFor({ timeout: 3000 });\n  const text = (await section.innerText()).toLowerCase();\n\n  if (!text.includes("1.8.6")) throw new Error("the section does not name the version");\n\n  const claims = [\n    [/arp -n -a|numeric.*arp/, "numeric macOS ARP discovery"],\n    [/reverse-dns|hostname lookup/, "reverse-DNS stall prevention"],\n    [/icmp|ping/, "numeric ICMP probing"],\n    [/positive icmp\\/tcp|positive.*icmp|positive.*tcp/, "positive probe retention"],\n    [/proxy-arp/, "proxy-ARP protection"],\n    [/send to arcatlas|single arcatlas toolbar action/, "simplified ArcAtlas action"],\n  ];\n  for (const [pattern, label] of claims) {\n    if (!pattern.test(text)) throw new Error(\`the section does not cover \${label}\`);\n  }\n\n  const headings = await section.locator("h3").allInnerTexts();\n  if (headings.length !== 3) throw new Error(\`expected 3 improvements, got \${headings.length}\`);\n  return headings.map((h) => h.trim()).join(", ");\n});`,
  "current release section verifier",
);
verify = verify.replace('await step("the What changed link opens the local 1.8.5 page"', 'await step("the What changed link opens the local 1.8.6 page"');
verify = replaceOnce(verify, 'if (href !== "whats-new-1.8.5.html")', 'if (href !== "whats-new-1.8.6.html")', "current What changed href");
verify = replaceOnce(
  verify,
  '    "whats-new-1.8.5.html",\n    "whats-new-1.8.4.html",',
  '    "whats-new-1.8.6.html",\n    "whats-new-1.8.5.html",\n    "whats-new-1.8.4.html",',
  "sitemap current release",
);
verify = replaceOnce(
  verify,
  '  const currentWhatsNew = "/whats-new-1.8.5.html";',
  '  const currentWhatsNew = "/whats-new-1.8.6.html";',
  "current What's New path",
);
verify = replaceOnce(
  verify,
  '  if (!/Observed inventory, straight into ArcAtlas/i.test(heading)) {',
  '  if (!/macOS LAN scans keep the devices they actually find/i.test(heading)) {',
  "current What's New heading",
);
verify = replaceOnce(
  verify,
  '  return "home and 1.8.5 release page link both ways";',
  '  return "home and 1.8.6 release page link both ways";',
  "current cross-link result",
);
verify = replaceOnce(
  verify,
  '    { label: "whats-new 1.8.5 desktop", path: "/whats-new-1.8.5.html", width: 1440, height: 900 },\n    { label: "whats-new 1.8.5 mobile", path: "/whats-new-1.8.5.html", width: 390, height: 844 },\n    // The previous release\'s page stays published, so it stays checked.\n    { label: "whats-new 1.8.3", path: "/whats-new-1.8.3.html", width: 1440, height: 900 },',
  '    { label: "whats-new 1.8.6 desktop", path: "/whats-new-1.8.6.html", width: 1440, height: 900 },\n    { label: "whats-new 1.8.6 mobile", path: "/whats-new-1.8.6.html", width: 390, height: 844 },\n    // The previous release\'s page stays published, so it stays checked.\n    { label: "whats-new 1.8.5", path: "/whats-new-1.8.5.html", width: 1440, height: 900 },',
  "axe current release matrix",
);
write("scripts/verify-site.mjs", verify);

console.log("Prepared ArcScan v1.8.6 release metadata and website.");
