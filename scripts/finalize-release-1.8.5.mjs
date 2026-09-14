#!/usr/bin/env node

import { readFileSync, writeFileSync } from "node:fs";

const read = (path) => readFileSync(path, "utf8");
const write = (path, value) => writeFileSync(path, value);

function replaceOnce(text, before, after, label) {
  if (text.includes(after)) return text;
  if (!text.includes(before)) throw new Error(`Could not find ${label}`);
  return text.replace(before, after);
}

// Keep package lock metadata aligned with package.json. The lockfile predates the
// recent release bumps, so only the root package versions need changing.
let packageLock = read("package-lock.json");
packageLock = packageLock.replace('"version": "1.8.1"', '"version": "1.8.5"');
packageLock = packageLock.replace('"version": "1.8.1"', '"version": "1.8.5"');
write("package-lock.json", packageLock);

// Cargo.lock carries ArcScan's local package version separately from Cargo.toml.
let cargoLock = read("src-tauri/Cargo.lock");
cargoLock = cargoLock.replace(
  /(\[\[package\]\]\nname = "arcscan"\nversion = ")[^"]+("\n)/,
  "$11.8.5$2",
);
write("src-tauri/Cargo.lock", cargoLock);

// Add today's macOS reliability fix to the existing ArcAtlas-focused release page.
let whatsNew = read("site/whats-new-1.8.5.html");
whatsNew = replaceOnce(
  whatsNew,
  `        <h2>Small setup improvements</h2>`,
  `        <h2>More reliable macOS discovery</h2>\n        <p>\n          A device that genuinely answered ICMP or TCP now stays in the final scan even if macOS\n          has omitted or aged its ARP cache entry before enrichment finishes. Missing ARP data is\n          treated as missing enrichment, not proof that a responsive device vanished.\n        </p>\n        <p>\n          The safety boundary stays intact: a present ARP entry identified as a proxy responder is\n          still rejected, while a legitimate ARP/MAC entry can still keep a quiet local device that\n          ignored active probes. Regression tests cover all three cases.\n        </p>\n\n        <h2>Small setup improvements</h2>`,
  "macOS reliability section",
);
write("site/whats-new-1.8.5.html", whatsNew);

// Surface the reliability improvement on the homepage without displacing the
// headline ArcAtlas feature that was already prepared for 1.8.5.
let home = read("site/index.html");
home = replaceOnce(
  home,
  `            <article>\n              <h3>Clearer build identity</h3>\n              <p>\n                The main header now shows the ArcScan version, and CI test builds can include the\n                short source commit. Pasting the full ArcAtlas machine endpoint is also normalized\n                automatically to the server URL before connection validation.\n              </p>\n            </article>`,
  `            <article>\n              <h3>More reliable macOS discovery</h3>\n              <p>\n                Devices that answer ICMP or TCP no longer disappear at finalization just because\n                macOS dropped the matching ARP cache entry. Proxy-ARP false-positive protection and\n                quiet-device ARP discovery remain intact.\n              </p>\n            </article>`,
  "homepage third 1.8.5 card",
);
write("site/index.html", home);

// Close 1.8.4 and document the complete 1.8.5 release: ArcAtlas handoff plus the
// macOS scanner reliability fix merged today.
let changelog = read("CHANGELOG.md");
if (!changelog.includes("## [1.8.5] - 2026-09-14")) {
  const section = `## [1.8.5] - 2026-09-14\n\nArcAtlas handoff plus a macOS discovery reliability fix. ArcScan can explicitly\nsend one selected network inventory to an ArcAtlas Discovery inbox, and responsive\nmacOS hosts no longer disappear during finalization when neighbor-cache enrichment\nis missing. Full notes:\n[docs/RELEASE-NOTES-1.8.5.md](docs/RELEASE-NOTES-1.8.5.md).\n\n### Added\n\n- **Explicit ArcAtlas handoff from Inventory.** Configure a server and site-scoped\n  token, choose one network, review the destination and device count, then confirm\n  **Send to ArcAtlas**. Nothing is sent automatically when a scan completes.\n- **Secure ArcAtlas token handling.** Installed ArcScan stores the token in the OS\n  credential store; Portable keeps it in process memory for the current session.\n  The token is not returned to the UI after setup, logged, or placed in a URL.\n- **Idempotent retry behavior** so uncertain network failures can be retried without\n  accidentally creating duplicate Discovery runs.\n- **Visible build identity and safer ArcAtlas URL setup.** The app exposes its\n  version/build identity and normalizes a pasted machine endpoint back to the\n  server URL before validation.\n\n### Fixed\n\n- **macOS devices no longer appear live and then disappear at the end of a scan**\n  when their ARP/neighbor-cache entry is missing during final enrichment. A positive\n  ICMP or TCP response remains authoritative evidence that the host is alive.\n- **Proxy-ARP filtering remains strict.** If an ARP entry is present but its MAC is\n  classified as a proxy responder, the apparent host is still rejected.\n- **Quiet local devices remain discoverable through a legitimate ARP/MAC entry**\n  even when they ignore active ICMP/TCP probes.\n- Added regression coverage for all three liveness cases so live results and saved\n  history cannot drift apart again.\n\n### Website and privacy\n\n- Updated the first-party **What's new in 1.8.5** page and homepage release summary\n  to cover both ArcAtlas handoff and the macOS reliability fix.\n- Expanded the privacy copy to document exactly when ArcAtlas receives inventory,\n  what is sent, and how Installed versus Portable stores the connection secret.\n\n`;
  changelog = changelog.replace("## [1.8.4] - unreleased", section + "## [1.8.4] - 2026-08-27");
}
write("CHANGELOG.md", changelog);

// Keep the sitemap in step with the newly published release page.
let sitemap = read("site/sitemap.xml");
sitemap = sitemap.replace("<lastmod>2026-08-26</lastmod>", "<lastmod>2026-09-14</lastmod>");
if (!sitemap.includes("whats-new-1.8.5.html")) {
  const marker = `  <url>\n    <loc>https://kingnazz.github.io/ArcScan/whats-new-1.8.4.html</loc>`;
  const entry = `  <url>\n    <loc>https://kingnazz.github.io/ArcScan/whats-new-1.8.5.html</loc>\n    <lastmod>2026-09-14</lastmod>\n    <changefreq>yearly</changefreq>\n    <priority>0.8</priority>\n  </url>\n`;
  if (!sitemap.includes(marker)) throw new Error("Could not find sitemap insertion point");
  sitemap = sitemap.replace(marker, entry + marker);
}
write("site/sitemap.xml", sitemap);

// A version bump merged into main should publish automatically. Normal app/code
// merges do not run the release workflow unless package.json itself changed.
let releaseWorkflow = read(".github/workflows/release.yml");
if (!releaseWorkflow.includes("      - package.json")) {
  releaseWorkflow = releaseWorkflow.replace(
    "on:\n  workflow_dispatch:\n",
    "on:\n  workflow_dispatch:\n  push:\n    branches:\n      - main\n    paths:\n      - package.json\n",
  );
}
releaseWorkflow = releaseWorkflow.replace(
  "# Manually publish a SIGNED release with installers + the auto-updater manifest",
  "# Publish a SIGNED release with installers + the auto-updater manifest",
);
releaseWorkflow = releaseWorkflow.replace(
  '# (latest.json) for all platforms. Actions tab -> "Publish Release" -> Run\n# workflow. The version/tag is read automatically from package.json, so it',
  '# (latest.json) for all platforms. A package.json version bump on main publishes\n# automatically; workflow_dispatch remains available for recovery. The version/tag',
);
write(".github/workflows/release.yml", releaseWorkflow);

const releaseNotes = `# ArcScan 1.8.5\n\nArcScan 1.8.5 combines a direct ArcAtlas Discovery handoff with a reliability fix\nfor local network discovery on macOS.\n\n## ArcAtlas direct handoff\n\n- Choose one Inventory network and explicitly send its observed inventory to a\n  configured ArcAtlas Discovery inbox. Nothing is uploaded merely because a scan\n  completes.\n- Installed ArcScan stores the connection token in the operating system credential\n  store. Portable keeps it in process memory for the current session only.\n- Retry uses a stable handoff identifier after uncertain failures so ArcAtlas can\n  return the existing run instead of creating a duplicate.\n- Pasting the full ArcAtlas machine endpoint is normalized to the server URL before\n  validation, and the app exposes clearer version/build identity.\n\n## macOS scan reliability\n\n- A host that positively answers ICMP or TCP now remains in the final result even\n  if macOS drops or omits its ARP entry before final enrichment.\n- A present proxy-ARP entry is still rejected, preserving protection against router\n  or access-point false positives.\n- Legitimate ARP-only quiet devices are still retained.\n- Regression tests cover all three paths so streamed results and saved history stay\n  consistent.\n\n## Verification\n\nThe macOS fix passed Rust formatting, unit tests, Clippy, Portable-edition tests and\nClippy, frontend tests/build, browser verification, and the universal macOS installer\nbuild before merge. The final 1.8.5 release is rebuilt and signed from the release\ncommit by GitHub Actions.\n\n## Upgrade\n\nInstalled ArcScan can use the normal in-app updater. Windows Portable users should\nfinish the current session, export anything they want to retain, close ArcScan, and\ndownload the new Portable ZIP.\n`;
write("docs/RELEASE-NOTES-1.8.5.md", releaseNotes);

console.log("ArcScan 1.8.5 release metadata reconciled.");
