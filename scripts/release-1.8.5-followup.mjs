#!/usr/bin/env node

import { readFileSync, writeFileSync } from "node:fs";

function replaceExact(text, before, after, label) {
  if (!text.includes(before)) throw new Error(`Could not find ${label}`);
  return text.replace(before, after);
}

let index = readFileSync("site/index.html", "utf8");
index = replaceExact(
  index,
  "              Network data stays on your computer",
  "              Local by default; ArcAtlas sends are explicit",
  "hero local-first fact",
);
index = replaceExact(
  index,
  `          Free and open source, for Windows and macOS. Your scan results never leave your computer.`,
  `          Free and open source, for Windows and macOS. Scan data stays local unless you explicitly send a selected network inventory to ArcAtlas.`,
  "final local-first CTA",
);
writeFileSync("site/index.html", index);

let readme = readFileSync("README.md", "utf8");
readme = replaceExact(
  readme,
  `application: everything runs on your own computer, there is no account, no cloud\nservice and no subscription, and nothing it finds is uploaded anywhere.`,
  `application: everything runs on your own computer, there is no ArcScan account or\nsubscription, and scan data stays local unless you explicitly send one selected\nnetwork inventory to a configured ArcAtlas server.`,
  "README introduction",
);
readme = replaceExact(
  readme,
  `Scanning is entirely local. Scan results, the device inventory, your names and\nyour notes are written to a SQLite file on your computer and are never sent\nanywhere.\n\nInstalled ArcScan can make one optional request on its own, and both editions\noffer one operator-triggered lookup:`,
  `Scanning itself is entirely local. Scan results, the device inventory, your names\nand your notes are written to a SQLite file on your computer. Nothing is uploaded\nin the background. Version 1.8.5 adds one explicit exception: after you configure\nan ArcAtlas server, choose one network and confirm Send to ArcAtlas, that selected\nnetwork inventory is sent to the configured server's Discovery inbox.\n\nInstalled ArcScan can make one optional request on its own, and both editions\noffer operator-triggered network actions:`,
  "README network request introduction",
);
readme = replaceExact(
  readme,
  `- **Public IP lookup**, which **only runs when you press Check** on the Scan\n  screen. It contacts \`api64.ipify.org\`, then \`icanhazip.com\`, sends nothing but\n  the request, and keeps the answer in memory for the session only. It is never\n  looked up at startup, after a scan, on a view change or on a timer, and it can\n  be switched off entirely in Settings.`,
  `- **Public IP lookup**, which **only runs when you press Check** on the Scan\n  screen. It contacts \`api64.ipify.org\`, then \`icanhazip.com\`, sends nothing but\n  the request, and keeps the answer in memory for the session only. It is never\n  looked up at startup, after a scan, on a view change or on a timer, and it can\n  be switched off entirely in Settings.\n- **ArcAtlas direct handoff**, which runs only after you configure an ArcAtlas\n  server, choose one network in Inventory, review the destination and device\n  count, and confirm **Send to ArcAtlas**. Installed stores the token in the OS\n  credential store; Portable keeps it in process memory only.`,
  "README network request bullets",
);
writeFileSync("README.md", readme);

console.log("Applied v1.8.5 local-first wording follow-up.");
