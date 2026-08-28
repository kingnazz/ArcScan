#!/usr/bin/env node
// Launch the real Windows x64 binaries and verify Portable's disposable-session
// contract from outside the process.
//
// Rust unit tests prove each cleanup predicate in isolation. This script covers
// the integration boundary they cannot: the compiled executable must choose a
// fresh system-temp SQLite database and WebView2 profile for every process,
// allow concurrent Portable processes, clean only ArcScan-owned sessions, and
// leave the executable folder, Installed AppData, and explicit exports alone.
//
//   node scripts/verify-portable-runtime.mjs --portable <portable x64 exe>
//                                            --installed <installed x64 exe>
//
// The build and release workflows run this only on windows-latest. A non-Windows
// invocation is an explicit skip because neither Wine nor a cross-compiled
// binary can prove WebView2 profile placement or Windows file-lock behaviour.

import { spawn, spawnSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

const args = process.argv.slice(2);
const flag = (name) => {
  const index = args.indexOf(`--${name}`);
  return index >= 0 ? args[index + 1] : null;
};

const portableExe = flag("portable");
const installedExe = flag("installed");
if (!portableExe || !installedExe) {
  console.error(
    "usage: verify-portable-runtime.mjs --portable <portable x64 exe> --installed <installed x64 exe>",
  );
  process.exit(1);
}
if (process.platform !== "win32") {
  console.log("Portable runtime verification is Windows-only; skipping on this host.");
  process.exit(0);
}
if (!existsSync(portableExe) || !existsSync(installedExe)) {
  console.error(
    `missing executable: portable=${existsSync(portableExe)} installed=${existsSync(installedExe)}`,
  );
  process.exit(1);
}

const RUN_MS = Number(process.env.ARCSCAN_VERIFY_RUN_MS || 25000);
const POLL_MS = 250;
const root = mkdtempSync(path.join(tmpdir(), "arcscan-disposable-runtime-"));
const launchCwd = path.join(root, "unrelated-working-directory");
const systemTemp = path.join(root, "system-temp");
const namespaceRoot = path.join(systemTemp, "ArcScanPortable");
const sessionsRoot = path.join(namespaceRoot, "sessions");
const portableFolder = path.join(root, "read-only-portable-folder");
const installedFolder = path.join(root, "installed-folder");
const exportFolder = path.join(root, "exports-outside-the-session");
const installedAppIdentifier = "com.arcscan.app";
const markerName = ".arcscan-portable-session";
const lockName = ".arcscan-portable-session.lock";
const exeName = "ArcScan.exe";
const ownedUnknownId = "22222222222242228222222222222222";
const ownedJunctionId = "33333333333343338333333333333333";
const adversarialFixtureIds = new Set([ownedUnknownId, ownedJunctionId]);

let failures = 0;
const children = new Set();
const writeDenied = new Set();

function check(ok, description, detail) {
  if (ok) {
    console.log(`  ok    ${description}`);
    return;
  }
  failures += 1;
  console.log(`  FAIL  ${description}${detail ? `\n        ${detail}` : ""}`);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitFor(description, predicate, timeout = RUN_MS) {
  const deadline = Date.now() + timeout;
  let lastError = null;
  while (Date.now() < deadline) {
    try {
      const value = predicate();
      if (value) return value;
    } catch (error) {
      lastError = error;
    }
    await sleep(POLL_MS);
  }
  throw new Error(
    `timed out waiting for ${description}${lastError ? `: ${lastError.message}` : ""}`,
  );
}

function list(dir) {
  try {
    return readdirSync(dir).sort();
  } catch {
    return null;
  }
}

function hashFile(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex");
}

/** A deterministic recursive snapshot without following links. */
function treeSnapshot(rootPath) {
  if (!existsSync(rootPath)) return null;
  const rows = [];
  const walk = (dir, prefix = "") => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
      const absolute = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        rows.push(`${relative}/`);
        walk(absolute, relative);
      } else if (entry.isFile()) {
        const stats = statSync(absolute);
        rows.push(`${relative}:${stats.size}:${hashFile(absolute)}`);
      } else {
        rows.push(`${relative}:other`);
      }
    }
  };
  walk(rootPath);
  return rows.sort();
}

function assertX64Pe(file, label) {
  const buffer = readFileSync(file);
  const peOffset = buffer.length > 0x40 ? buffer.readUInt32LE(0x3c) : 0;
  const valid =
    buffer.length > peOffset + 6 &&
    buffer.readUInt16LE(0) === 0x5a4d &&
    buffer.readUInt32LE(peOffset) === 0x00004550;
  check(valid, `${label} is a PE executable`);
  if (valid) {
    const machine = buffer.readUInt16LE(peOffset + 4);
    check(
      machine === 0x8664,
      `${label} has the exact x64 PE machine type`,
      `0x${machine.toString(16)}`,
    );
  }
  return buffer;
}

function currentSid() {
  const result = spawnSync(
    "powershell",
    [
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      "[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value",
    ],
    { encoding: "utf8" },
  );
  if (result.status !== 0 || !result.stdout.trim()) {
    throw new Error(`could not resolve the current Windows SID: ${result.stderr || result.stdout}`);
  }
  return `*${result.stdout.trim()}`;
}

const sid = currentSid();

function roamingAppData() {
  const result = spawnSync(
    "powershell",
    [
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      "[Environment]::GetFolderPath([Environment+SpecialFolder]::ApplicationData)",
    ],
    { encoding: "utf8" },
  );
  if (result.status !== 0 || !result.stdout.trim()) {
    throw new Error(
      `could not resolve Windows Roaming AppData: ${result.stderr || result.stdout}`,
    );
  }
  return path.resolve(result.stdout.trim());
}

// Tauri resolves FOLDERID_RoamingAppData through the Windows known-folder API;
// overriding APPDATA in a child environment does not redirect that API. The
// native coexistence test therefore uses the real resolved location, but only
// on a clean runner/profile where ArcScan's exact directory does not exist. A
// private marker proves this harness created the directory before it removes it.
const roamingData = roamingAppData();
const installedData = path.join(roamingData, installedAppIdentifier);
const installedFixtureMarker = path.join(installedData, ".arcscan-runtime-verifier");
const installedFixtureRecord = `${JSON.stringify({
  product: "ArcScan",
  kind: "installed-runtime-verifier",
  token: randomUUID(),
})}\n`;

function exactInstalledFixturePath() {
  return (
    path.basename(installedData).toLowerCase() === installedAppIdentifier &&
    path.resolve(path.dirname(installedData)).toLowerCase() === roamingData.toLowerCase()
  );
}

function prepareInstalledDataFixture() {
  if (!exactInstalledFixturePath()) {
    throw new Error(`refusing an unexpected Installed data path: ${installedData}`);
  }
  if (existsSync(installedData)) {
    throw new Error(
      `refusing to run over existing Installed ArcScan data: ${installedData}`,
    );
  }
  mkdirSync(installedData);
  writeFileSync(installedFixtureMarker, installedFixtureRecord);
}

function removeInstalledDataFixture() {
  if (!existsSync(installedData)) return;
  if (
    !exactInstalledFixturePath() ||
    !existsSync(installedFixtureMarker) ||
    readFileSync(installedFixtureMarker, "utf8") !== installedFixtureRecord
  ) {
    throw new Error(`refusing to remove an unowned Installed data path: ${installedData}`);
  }
  rmSync(installedData, { recursive: true, force: true });
}

/**
 * Add an inheritable explicit write/delete deny while preserving read/execute.
 *
 * Do not use icacls' `(W)` shorthand here. Windows expands generic write to a
 * mask that includes SYNCHRONIZE, which is also needed when CreateProcess maps
 * an executable. Denying only the concrete mutation rights makes the fixture
 * genuinely non-writable without accidentally making ArcScan.exe unlaunchable.
 */
function denyWrites(folder) {
  const result = spawnSync(
    "icacls",
    [folder, "/deny", `${sid}:(OI)(CI)(WD,AD,WEA,WA,DC,DE)`, "/T", "/C", "/Q"],
    { encoding: "utf8" },
  );
  if (result.status !== 0) {
    throw new Error(`could not make ${folder} read-only: ${result.stderr || result.stdout}`);
  }
  writeDenied.add(folder);
}

function restoreWrites(folder) {
  if (!writeDenied.has(folder)) return;
  const result = spawnSync(
    "icacls",
    [folder, "/remove:d", sid, "/T", "/C", "/Q"],
    { encoding: "utf8" },
  );
  if (result.status !== 0) {
    console.error(`could not restore write access to ${folder}: ${result.stderr || result.stdout}`);
  } else {
    writeDenied.delete(folder);
  }
}

function launch(folder, label, environment = {}) {
  const child = spawn(path.join(folder, exeName), [], {
    cwd: launchCwd,
    env: {
      ...process.env,
      TEMP: systemTemp,
      TMP: systemTemp,
      ...environment,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  const state = { exited: false, code: null, output: "" };
  child.stdout.on("data", (chunk) => {
    state.output += chunk.toString();
  });
  child.stderr.on("data", (chunk) => {
    state.output += chunk.toString();
  });
  child.on("error", (error) => {
    state.output += `\n${error}`;
  });
  child.on("exit", (code) => {
    state.exited = true;
    state.code = code;
    children.delete(child);
  });
  child.arcscan = { label, state };
  children.add(child);
  return child;
}

function forceStop(child) {
  if (!child?.pid || child.arcscan.state.exited) return;
  spawnSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });
}

async function closeNormally(child, timeout = RUN_MS) {
  if (child.arcscan.state.exited) return false;
  const result = spawnSync(
    "powershell",
    [
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      `$process = Get-Process -Id ${Number(child.pid)} -ErrorAction Stop; if (-not $process.CloseMainWindow()) { exit 2 }`,
    ],
    { encoding: "utf8" },
  );
  if (result.status !== 0) return false;
  try {
    await waitFor(
      `${child.arcscan.label} to exit normally`,
      () => child.arcscan.state.exited,
      timeout,
    );
    return true;
  } catch {
    return false;
  }
}

async function stopAndWait(child) {
  forceStop(child);
  try {
    await waitFor(`${child.arcscan.label} to stop`, () => child.arcscan.state.exited, 10000);
  } catch {
    // The final cleanup will make another best-effort tree kill.
  }
}

function parseOwnedMarker(id) {
  const file = path.join(sessionsRoot, id, markerName);
  try {
    const marker = JSON.parse(readFileSync(file, "utf8"));
    return marker.product === "ArcScan" &&
      marker.kind === "portable-session" &&
      marker.format === 1 &&
      marker.session_id === id &&
      Number.isInteger(marker.process_id) &&
      marker.process_id > 0 &&
      !Number.isNaN(Date.parse(marker.created_at))
      ? marker
      : null;
  } catch {
    return null;
  }
}

function writeOwnershipFixture(id, extraSetup) {
  const session = path.join(sessionsRoot, id);
  mkdirSync(session, { recursive: true });
  writeFileSync(path.join(session, lockName), "");
  writeFileSync(
    path.join(session, markerName),
    `${JSON.stringify({
      product: "ArcScan",
      kind: "portable-session",
      format: 1,
      session_id: id,
      created_at: new Date().toISOString(),
      process_id: process.pid,
    }, null, 2)}\n`,
  );
  extraSetup(session);
  return session;
}

function ownedSessionIds() {
  return (list(sessionsRoot) || [])
    .filter((name) => /^[0-9a-f]{32}$/.test(name) && parseOwnedMarker(name))
    .sort();
}

async function waitForNewSession(child, previousIds) {
  const previous = new Set(previousIds);
  return waitFor(`${child.arcscan.label} to create its Portable session`, () => {
    if (child.arcscan.state.exited) {
      throw new Error(
        `${child.arcscan.label} exited ${child.arcscan.state.code}: ${child.arcscan.state.output.slice(-500)}`,
      );
    }
    const added = ownedSessionIds().filter((id) => !previous.has(id));
    return added.length === 1 ? added[0] : false;
  });
}

async function waitForReadySession(child, id) {
  const rootPath = path.join(sessionsRoot, id);
  await waitFor(`${child.arcscan.label}'s SQLite database and WebView2 profile`, () => {
    if (child.arcscan.state.exited) {
      throw new Error(
        `${child.arcscan.label} exited ${child.arcscan.state.code}: ${child.arcscan.state.output.slice(-500)}`,
      );
    }
    const profile = path.join(rootPath, "WebView");
    return (
      existsSync(path.join(rootPath, "arcscan.db")) &&
      existsSync(path.join(rootPath, lockName)) &&
      existsSync(profile) &&
      (list(profile) || []).length > 0
    );
  });
  return {
    id,
    root: rootPath,
    database: path.join(rootPath, "arcscan.db"),
    webview: path.join(rootPath, "WebView"),
  };
}

async function launchPortable(label) {
  const before = ownedSessionIds();
  const child = launch(portableFolder, label);
  const id = await waitForNewSession(child, before);
  const session = await waitForReadySession(child, id);
  return { child, session };
}

function portableFolderIsUntouched(before) {
  return JSON.stringify(treeSnapshot(portableFolder)) === JSON.stringify(before);
}

mkdirSync(launchCwd, { recursive: true });
mkdirSync(systemTemp, { recursive: true });
mkdirSync(sessionsRoot, { recursive: true });
mkdirSync(portableFolder, { recursive: true });
mkdirSync(installedFolder, { recursive: true });
mkdirSync(exportFolder, { recursive: true });
copyFileSync(portableExe, path.join(portableFolder, exeName));
copyFileSync(installedExe, path.join(installedFolder, exeName));

const portableBytes = assertX64Pe(path.join(portableFolder, exeName), "the Portable executable");
assertX64Pe(path.join(installedFolder, exeName), "the Installed executable");
const binaryText = portableBytes.toString("latin1");
for (const marker of ["tauri-plugin-updater", "plugin:updater", "download_and_install", "minisign"]) {
  check(!binaryText.includes(marker), `Portable carries no updater application path (no "${marker}")`);
}

// These files stand in for explicit save-dialog destinations. They are outside
// ArcScanPortable, so session cleanup has no authority to touch them. Command
// tests separately exercise save_text itself.
const exports = new Map([
  [path.join(exportFolder, "inventory.csv"), "address,name\n192.0.2.10,router\n"],
  [path.join(exportFolder, "inventory.json"), '{"devices":[{"address":"192.0.2.10"}]}\n'],
  [path.join(exportFolder, "inventory.xml"), '<inventory><device address="192.0.2.10"/></inventory>\n'],
]);
for (const [file, contents] of exports) writeFileSync(file, contents);

// Invalid names, a compact-UUID directory without a marker, and a sibling of
// sessions are all intentionally outside ArcScan cleanup authority.
const unknownInvalid = path.join(sessionsRoot, "keep-this-unknown-folder");
// Syntactically valid compact UUID v4 + RFC variant, but deliberately without
// ArcScan's ownership marker. Name validation alone must never authorize it.
const unknownUuid = path.join(sessionsRoot, "11111111111141118111111111111111");
const unknownSibling = path.join(namespaceRoot, "keep-this-namespace-sibling");
for (const dir of [unknownInvalid, unknownUuid, unknownSibling]) {
  mkdirSync(dir, { recursive: true });
  writeFileSync(path.join(dir, "sentinel.txt"), "not owned by ArcScan\n");
}

// These two candidates pass the namespace, UUID, marker and inactive-lock
// gates. Cleanup must still refuse them at the payload boundary: one has an
// unknown file, and one has a WebView directory junction pointing outside the
// session. A junction does not require Windows developer mode or elevation.
const ownedUnknown = writeOwnershipFixture(ownedUnknownId, (session) => {
  writeFileSync(path.join(session, "do-not-delete.txt"), "unknown payload\n");
});
const junctionTarget = path.join(root, "junction-target-outside-sessions");
mkdirSync(junctionTarget, { recursive: true });
writeFileSync(path.join(junctionTarget, "sentinel.txt"), "external target\n");
const ownedJunction = writeOwnershipFixture(ownedJunctionId, (session) => {
  symlinkSync(junctionTarget, path.join(session, "WebView"), "junction");
});

const portableFolderBefore = treeSnapshot(portableFolder);
denyWrites(portableFolder);
prepareInstalledDataFixture();
const appDataBeforePortable = treeSnapshot(installedData);

let portableA;
let portableB;
let portableC;
let portableD;
let installed;

try {
  console.log("Portable disposable-session runtime verification (Windows x64)");
  console.log(`  isolated system temp: ${systemTemp}`);
  console.log(`  read-only executable folder: ${portableFolder}`);
  console.log(`  clean Installed data fixture: ${installedData}`);

  console.log("\n1. Two concurrent launches from the same read-only extracted folder");
  portableA = await launchPortable("Portable A");
  portableB = await launchPortable("Portable B");
  const sessionA = portableA.session;
  const sessionB = portableB.session;
  check(!portableA.child.arcscan.state.exited && !portableB.child.arcscan.state.exited, "both processes stay active simultaneously");
  check(sessionA.id !== sessionB.id, "each process receives a unique session id");
  check(sessionA.database !== sessionB.database, "each process receives a different SQLite database");
  check(sessionA.webview !== sessionB.webview, "each process receives a different WebView2 profile");
  check(
    sessionA.root.startsWith(sessionsRoot + path.sep) && sessionB.root.startsWith(sessionsRoot + path.sep),
    "both sessions are direct children of <system temp>/ArcScanPortable/sessions",
  );
  check(existsSync(sessionA.database) && existsSync(sessionB.database), "both independent SQLite databases are open in-session");
  check(
    existsSync(sessionA.webview) && existsSync(sessionB.webview),
    "both independent WebView2 profiles are written in-session",
  );
  check(portableFolderIsUntouched(portableFolderBefore), "the read-only executable folder is byte-for-byte untouched");
  check(!existsSync(path.join(portableFolder, "ArcScanData")), "no persistent ArcScanData folder is created beside ArcScan.exe");
  check(
    JSON.stringify(treeSnapshot(installedData)) === JSON.stringify(appDataBeforePortable),
    "Portable leaves Installed ArcScan AppData untouched",
  );
  check(existsSync(unknownInvalid) && existsSync(unknownUuid) && existsSync(unknownSibling), "startup preserves unknown temp directories without valid ownership metadata");
  check(
    existsSync(path.join(ownedUnknown, "do-not-delete.txt")),
    "startup refuses a marker-valid session with an unknown payload",
  );
  check(
    existsSync(ownedJunction) && existsSync(path.join(junctionTarget, "sentinel.txt")),
    "startup refuses a marker-valid WebView junction and preserves its external target",
  );

  console.log("\n2. Installed ArcScan coexists with the active Portable sessions");
  const portableIdsBeforeInstalled = ownedSessionIds();
  installed = launch(installedFolder, "Installed ArcScan");
  await waitFor("Installed ArcScan to start", () => {
    if (installed.arcscan.state.exited) {
      throw new Error(`Installed ArcScan exited ${installed.arcscan.state.code}: ${installed.arcscan.state.output.slice(-500)}`);
    }
    return existsSync(path.join(installedData, "arcscan.db"));
  });
  check(!portableA.child.arcscan.state.exited && !portableB.child.arcscan.state.exited, "Installed and both Portable processes run at the same time");
  check(
    JSON.stringify(ownedSessionIds()) === JSON.stringify(portableIdsBeforeInstalled),
    "Installed ArcScan creates no Portable temp session and leaves active ones alone",
  );
  check(!existsSync(path.join(installedFolder, "ArcScanData")), "Installed ArcScan creates no data beside its executable");
  // This assertion is about coexistence and storage isolation. Close the
  // headless runner window when Windows accepts the request, then use bounded
  // process teardown so an updater/WebView runner condition cannot consume the
  // remainder of the Portable lifecycle test.
  const installedClosed = await closeNormally(installed, 5000);
  if (!installedClosed) await stopAndWait(installed);
  check(
    installed.arcscan.state.exited,
    "Installed ArcScan is stopped after the coexistence check",
    installed.arcscan.state.output.slice(-500),
  );

  console.log("\n3. Normal shutdown removes only the owned temporary session");
  const aClosed = await closeNormally(portableA.child);
  check(aClosed, "Portable A accepts a normal window close", portableA.child.arcscan.state.output.slice(-500));
  if (aClosed) {
    try {
      await waitFor("Portable A's owned session to be removed", () => !existsSync(sessionA.root));
      check(true, "normal shutdown removes Portable A's SQLite database, WebView profile, marker, lock, and session root");
    } catch (error) {
      check(false, "normal shutdown removes Portable A's whole owned session", error.message);
    }
  }
  check(existsSync(sessionB.root), "normal cleanup preserves the other active Portable session");
  check(existsSync(unknownUuid) && existsSync(unknownInvalid), "normal cleanup preserves unknown session entries");

  console.log("\n4. The next launch starts fresh while another Portable process remains active");
  portableC = await launchPortable("Portable C");
  const sessionC = portableC.session;
  check(sessionC.id !== sessionA.id && sessionC.id !== sessionB.id, "the next launch allocates a fresh session instead of reopening either prior one");
  check(!existsSync(sessionA.root), "the prior normal session stays gone");
  check(existsSync(sessionB.root), "startup stale cleanup recognizes and preserves an active concurrent session");
  check(
    sessionC.database !== sessionA.database && sessionC.webview !== sessionA.webview,
    "the fresh launch cannot inherit the prior SQLite database or WebView-backed preferences",
  );

  console.log("\n5. A crash is recovered by strict stale-session cleanup");
  await stopAndWait(portableB.child);
  check(existsSync(sessionB.root), "forced termination leaves a real ArcScan-owned stale session fixture");
  portableD = await launchPortable("Portable D");
  try {
    await waitFor("the crashed owned session to be cleaned", () => !existsSync(sessionB.root));
    check(true, "the next startup removes the stale owned SQLite/WebView session");
  } catch (error) {
    check(false, "the next startup removes the stale owned session", error.message);
  }
  check(existsSync(sessionC.root), "stale cleanup preserves the concurrently active session");
  check(
    existsSync(unknownInvalid) &&
      existsSync(unknownUuid) &&
      existsSync(unknownSibling) &&
      existsSync(ownedUnknown) &&
      existsSync(ownedJunction) &&
      existsSync(path.join(junctionTarget, "sentinel.txt")),
    "stale cleanup refuses invalid names, missing markers, unknown payloads, reparse points, and paths outside sessions",
  );

  console.log("\n6. An unusable temp root never falls back to Installed AppData");
  const appDataBeforeFailure = treeSnapshot(installedData);
  const blockedTemp = path.join(root, "unwritable-system-temp");
  mkdirSync(blockedTemp, { recursive: true });
  denyWrites(blockedTemp);
  const refused = launch(portableFolder, "Portable with blocked temp", { TEMP: blockedTemp, TMP: blockedTemp });
  let reported = false;
  try {
    await waitFor(
      "the blocked-temp startup error",
      () => /could not create a temporary session|temporary session path/i.test(refused.arcscan.state.output),
      Math.min(RUN_MS, 15000),
    );
    reported = true;
  } catch {
    // The checks below report both observability and the no-fallback result.
  }
  check(reported, "an unwritable system temp root produces the Portable-specific fatal error", refused.arcscan.state.output.slice(-500));
  await stopAndWait(refused);
  restoreWrites(blockedTemp);
  check(
    JSON.stringify(treeSnapshot(installedData)) === JSON.stringify(appDataBeforeFailure),
    "failed Portable startup does not silently fall back to Installed AppData",
  );
  check(!existsSync(path.join(portableFolder, "ArcScanData")), "failed startup does not fall back beside the executable");

  console.log("\n7. Closing the remaining sessions retains only explicit exports and unknown temp data");
  for (const running of [portableC, portableD]) {
    const closed = await closeNormally(running.child);
    check(closed, `${running.child.arcscan.label} closes normally`, running.child.arcscan.state.output.slice(-500));
    if (closed) {
      try {
        await waitFor(`${running.child.arcscan.label}'s session cleanup`, () => !existsSync(running.session.root));
        check(true, `${running.child.arcscan.label}'s owned session is removed`);
      } catch (error) {
        check(false, `${running.child.arcscan.label}'s owned session is removed`, error.message);
      }
    }
  }
  const liveCreatedSessions = ownedSessionIds().filter((id) => !adversarialFixtureIds.has(id));
  check(
    liveCreatedSessions.length === 0,
    "no session created by a real Portable launch remains after normal shutdowns",
    JSON.stringify(liveCreatedSessions),
  );
  for (const [file, contents] of exports) {
    check(existsSync(file) && readFileSync(file, "utf8") === contents, `${path.extname(file).slice(1).toUpperCase()} exported outside the session survives cleanup`);
  }
  check(
    existsSync(unknownInvalid) &&
      existsSync(unknownUuid) &&
      existsSync(unknownSibling) &&
      existsSync(path.join(ownedUnknown, "do-not-delete.txt")) &&
      existsSync(path.join(junctionTarget, "sentinel.txt")),
    "unowned and unsafe marker-valid temp data remains after every cleanup pass",
  );
  check(portableFolderIsUntouched(portableFolderBefore), "the executable folder remains unchanged across every launch and cleanup");
} catch (error) {
  check(false, "the runtime verification completed", error.stack || error.message);
} finally {
  for (const child of [...children]) forceStop(child);
  await sleep(750);
  for (const folder of [...writeDenied]) restoreWrites(folder);

  try {
    removeInstalledDataFixture();
    check(true, "the verifier removes only its marker-owned Installed data fixture");
  } catch (error) {
    check(false, "the verifier safely removes its Installed data fixture", error.message);
  }

  rmSync(root, { recursive: true, force: true });
}

console.log("");
if (failures > 0) {
  console.error(`${failures} expectation(s) failed.`);
  process.exit(1);
}
console.log("Every expectation held.");
