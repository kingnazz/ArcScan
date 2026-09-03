#!/usr/bin/env node

import { readFileSync, writeFileSync } from "node:fs";

function replaceExact(text, before, after, label) {
  if (!text.includes(before)) {
    throw new Error(`Could not find ${label}`);
  }
  return text.replace(before, after);
}

let index = readFileSync("site/index.html", "utf8");

index = replaceExact(
  index,
  'href="whats-new-1.8.4.html"\n              >What changed in 1.8.4</a',
  'href="whats-new-1.8.5.html"\n              >What changed in 1.8.5</a',
  "hero release-notes link",
);

index = replaceExact(
  index,
  '"text": "No. Scan results, the device inventory, your names and your notes stay on your computer and are never sent anywhere. Installed ArcScan can make an optional update check against GitHub, which sends only the version being checked and can be switched off. Both editions offer a public IP lookup that only runs when you press the button. Portable updates are manual."',
  '"text": "By default, scan results, the device inventory, your names and your notes stay on your computer. If you explicitly connect ArcScan to an ArcAtlas server and confirm Send to ArcAtlas, the selected network inventory is sent to that server\'s Discovery inbox. Nothing is sent automatically after a scan. Installed ArcScan can make an optional update check against GitHub, and both editions offer a public IP lookup that only runs when you press the button. Portable updates are manual."',
  "structured privacy FAQ answer",
);

index = replaceExact(
  index,
  `              <p>\n                No. Scan results, the device inventory, your names and your notes are stored in a\n                SQLite database on your own computer and are never sent anywhere.\n              </p>`,
  `              <p>\n                By default, scan results, the device inventory, your names and your notes are stored\n                in a SQLite database on your own computer. If you explicitly connect ArcScan to an\n                ArcAtlas server and confirm <strong>Send to ArcAtlas</strong>, the selected network\n                inventory is sent to that server's Discovery inbox. Nothing is sent automatically\n                after a scan.\n              </p>`,
  "visible privacy FAQ answer",
);

index = replaceExact(
  index,
  `              <p>\n                The inventory, the changes, your device names and your notes live in a SQLite file\n                on your own computer. Nothing is uploaded, and there is no account to create.\n              </p>`,
  `              <p>\n                The inventory, the changes, your device names and your notes live in a SQLite file\n                on your own computer. They stay local unless you explicitly send a selected network\n                inventory to a configured ArcAtlas server. There is no ArcScan account to create.\n              </p>`,
  "local-first benefit copy",
);

index = replaceExact(
  index,
  `              <p>\n                No password is ever attempted. No exploit is ever sent. No scan result, device name\n                or note leaves your computer. There is no stealth or evasion behaviour, because\n                ArcScan is a network administration tool and nothing else.\n              </p>`,
  `              <p>\n                No password is ever attempted. No exploit is ever sent. ArcScan never uploads scan\n                data in the background. A selected network inventory leaves your computer only when\n                you explicitly confirm <strong>Send to ArcAtlas</strong> for a configured server.\n                There is no stealth or evasion behaviour.\n              </p>`,
  "never-does privacy copy",
);

index = index.replace(
  /      <!-- ====================================================== new in 1\.8\.4 -->[\s\S]*?      <section class="section" id="download">/,
  `      <!-- ====================================================== new in 1.8.5 -->\n      <section class="section" id="whats-new">\n        <div class="wrap">\n          <div class="section-head">\n            <p class="eyebrow">New in 1.8.5</p>\n            <h2>Send observed inventory straight to ArcAtlas.</h2>\n            <p>\n              ArcScan can now hand one selected network directly to an ArcAtlas Discovery inbox.\n              The send is always explicit, reuses ArcScan's existing Inventory JSON shape, and never\n              creates or edits documented devices on its own.\n              <a href="whats-new-1.8.5.html">What&rsquo;s new in 1.8.5</a>\n            </p>\n          </div>\n\n          <div class="whats-new">\n            <article>\n              <h3>One network, one deliberate send</h3>\n              <p>\n                Choose a network in Inventory, review the destination and device count, then confirm\n                Send to ArcAtlas. Search, presence and device-type filters never silently shrink the\n                snapshot, and nothing is sent when a scan merely finishes.\n              </p>\n            </article>\n\n            <article>\n              <h3>Secrets stay out of the interface</h3>\n              <p>\n                Installed ArcScan stores the connection token in the operating system credential\n                store. Portable keeps it in process memory for the current session only. The token\n                is never returned to the interface after setup and is never placed in a URL.\n              </p>\n            </article>\n\n            <article>\n              <h3>Clearer build identity</h3>\n              <p>\n                The main header now shows the ArcScan version, and CI test builds can include the\n                short source commit. Pasting the full ArcAtlas machine endpoint is also normalized\n                automatically to the server URL before connection validation.\n              </p>\n            </article>\n          </div>\n        </div>\n      </section>\n\n      <section class="section" id="download">`,
);

if (!index.includes("New in 1.8.5") || !index.includes("whats-new-1.8.5.html")) {
  throw new Error("The v1.8.5 What's New section was not applied");
}

writeFileSync("site/index.html", index);

let privacy = readFileSync("site/privacy.html", "utf8");

privacy = replaceExact(
  privacy,
  `          Your scan results, device inventory, names and notes never leave your computer. The\n          Installed edition can make an optional update check. Both editions offer a public IP\n          lookup that only runs when you press <strong>Check</strong>. Portable updates are manual.`,
  `          Your scan results, device inventory, names and notes stay on your computer by default.\n          If you explicitly connect ArcScan to an ArcAtlas server and confirm a send, the selected\n          network inventory is sent to that server's Discovery inbox. Nothing is sent automatically\n          after a scan. The Installed edition can make an optional update check. Both editions offer\n          a public IP lookup that only runs when you press <strong>Check</strong>.`,
  "privacy short version",
);

privacy = replaceExact(
  privacy,
  `          a fresh private database for the current process and removes it during safe session\n          cleanup. Nothing in either database is transmitted anywhere.`,
  `          a fresh private database for the current process and removes it during safe session\n          cleanup. Database contents are not transmitted in the background. The explicit ArcAtlas\n          handoff described below is the only feature that sends selected inventory data to a server.`,
  "privacy local database statement",
);

privacy = replaceExact(
  privacy,
  `        <h3>Update check</h3>`,
  `        <h3>ArcAtlas direct handoff</h3>\n        <p>\n          Version 1.8.5 adds an optional direct handoff to ArcAtlas. ArcScan never sends inventory\n          merely because a scan completed. A technician must first configure an ArcAtlas server and\n          connection token, choose one network in Inventory, review the destination and device count,\n          and press <strong>Send to ArcAtlas</strong>.\n        </p>\n        <p>\n          That explicit action sends the selected network's existing Inventory JSON fields over\n          HTTPS to <code>/api/discovery/arcscan</code> on the server you configured. The ArcAtlas\n          server therefore receives the inventory data in that handoff and ordinary HTTPS request\n          metadata such as your public IP address. ArcScan does not contact an ArcAtlas server in the\n          background, on a timer, or after a scan without that confirmation.\n        </p>\n        <p>\n          Installed ArcScan stores the ArcAtlas connection token in the operating system credential\n          store and keeps only non-secret destination metadata in its application-data directory.\n          Portable stores both the token and connection metadata in process memory only, so they\n          disappear when the Portable process exits. The full token is never returned to the webview\n          after setup, never logged, and never placed in a URL. Disconnecting locally removes the\n          local secret but does not revoke the token on the ArcAtlas server.\n        </p>\n\n        <h3>Update check</h3>`,
  "ArcAtlas privacy subsection",
);

privacy = replaceExact(
  privacy,
  `          <li>No upload of scan results, device names, notes, MAC addresses or hostnames.</li>`,
  `          <li>No background or automatic upload of scan results. ArcAtlas inventory is sent only after an explicit technician-confirmed handoff.</li>`,
  "never-collects upload bullet",
);

writeFileSync("site/privacy.html", privacy);

const whatsNew = `<!doctype html>\n<html lang="en">\n  <head>\n    <meta charset="utf-8" />\n    <meta name="viewport" content="width=device-width, initial-scale=1" />\n    <title>What's new in ArcScan 1.8.5</title>\n    <meta name="description" content="ArcScan 1.8.5 adds an explicit direct handoff from Inventory to an ArcAtlas Discovery inbox, secure connection-token storage, safer URL setup and visible build identity." />\n    <link rel="canonical" href="https://kingnazz.github.io/ArcScan/whats-new-1.8.5.html" />\n    <link rel="icon" href="assets/favicon.png" />\n    <meta name="color-scheme" content="dark" />\n    <meta name="theme-color" content="#000000" />\n    <meta http-equiv="Content-Security-Policy" content="default-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'none'; connect-src 'self'; base-uri 'none'; form-action 'none'" />\n    <link rel="stylesheet" href="arcscan.v5.css" />\n  </head>\n  <body>\n    <a class="skip-link" href="#main">Skip to content</a>\n    <header class="nav">\n      <div class="nav-inner">\n        <a class="brand" href="./"><img src="assets/logo.png" width="24" height="24" alt="" /><span>ArcScan</span></a>\n        <nav aria-label="Primary"><ul class="nav-links"><li><a href="./#features">Features</a></li><li><a href="./#download">Download</a></li><li><a href="privacy.html">Privacy</a></li><li><a href="https://github.com/kingnazz/ArcScan" rel="noopener">GitHub</a></li></ul></nav>\n      </div>\n    </header>\n    <main id="main" class="section">\n      <div class="wrap prose">\n        <p class="eyebrow">ArcScan 1.8.5</p>\n        <h1 class="page-title">Observed inventory, straight into ArcAtlas.</h1>\n        <p class="lede">\n          1.8.5 adds a direct, technician-controlled handoff from ArcScan Inventory to an ArcAtlas\n          Discovery inbox. It removes the export-and-upload step without turning ArcScan into a\n          background agent or changing what Discovery is allowed to do.\n        </p>\n\n        <h2>Send one network directly from Inventory</h2>\n        <p>\n          Connect ArcScan to a site-scoped ArcAtlas token, choose one network, review the destination\n          and device count, and confirm <strong>Send to ArcAtlas</strong>. ArcScan sends the same row\n          shape its existing Inventory JSON export already produces. Search text, presence filters,\n          device-type filters and sorting do not silently remove devices from the handoff snapshot.\n        </p>\n        <p>\n          Nothing is sent when a scan completes. ArcAtlas receives observed evidence in Discovery;\n          it does not silently create or overwrite documented devices.\n        </p>\n\n        <h2>Connection secrets stay in the right place</h2>\n        <p>\n          Installed ArcScan keeps the connection token in the operating system credential store.\n          Portable keeps it in memory for the current process only. After setup, the full token is\n          never returned to the interface, written to the ArcScan database, placed in a URL or\n          included in diagnostics.\n        </p>\n        <p>\n          ArcScan accepts HTTPS servers, with plain HTTP limited to local development on loopback.\n          Redirects are refused for bearer-token requests.\n        </p>\n\n        <h2>Retries do not create accidental duplicate runs</h2>\n        <p>\n          If a request times out or has an uncertain network failure, Retry reuses the same handoff\n          identifier so ArcAtlas can return the already-created run. After a confirmed success, the\n          next deliberate send receives a new identifier and creates a new historical Discovery run.\n        </p>\n\n        <h2>Small setup improvements</h2>\n        <p>\n          If you paste the full ArcAtlas machine endpoint, ArcScan now normalizes it back to the\n          server URL before validation. The main header also shows the application version, and CI\n          test builds can include the short source commit so it is obvious which binary is running.\n        </p>\n\n        <div class="notice">\n          <div>\n            <h3>The local-first boundary is explicit</h3>\n            <p>\n              ArcScan still has no account, telemetry or automatic scan-data upload. The ArcAtlas\n              handoff is an optional outbound action that occurs only after a technician configures\n              a server and confirms a send. The <a href="privacy.html">privacy page</a> documents the\n              exact boundary.\n            </p>\n          </div>\n        </div>\n\n        <div class="cta-row">\n          <a class="btn btn-primary" href="./#download">Download ArcScan</a>\n          <a class="btn btn-secondary" href="https://github.com/kingnazz/ArcScan/releases" rel="noopener">Release history</a>\n        </div>\n      </div>\n    </main>\n    <footer class="footer"><div class="wrap"><p class="footer-note">ArcScan is free and open source under the MIT license. Scan only networks you own or are authorised to inspect.</p></div></footer>\n  </body>\n</html>\n`;

writeFileSync("site/whats-new-1.8.5.html", whatsNew);

console.log("Prepared ArcScan 1.8.5 website and privacy copy.");
