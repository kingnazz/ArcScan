#!/usr/bin/env node
// Launch a real portable ArcScan and check where it actually puts its data.
//
// Everything about portable mode is a claim about the filesystem, and the only
// way to check a claim about the filesystem is to run the binary and look. The
// Rust tests cover the path arithmetic and the lock semantics in isolation; this
// covers the thing they cannot: that a packaged executable, started from an
// unrelated working directory, creates ArcScanData beside itself, keeps its
// database and its WebView profile in there, refuses a second copy from the same
// folder, and leaves both the other portable folder and the installed
// application-data directory alone.
//
//   node scripts/verify-portable-runtime.mjs --portable <path to portable exe>
//                                            [--installed <path to installed exe>]
//
// Exits non-zero on the first failed expectation.

import { spawn, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync, statSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import path from "node:path";

const args = process.argv.slice(2);
const flag = (name) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? args[i + 1] : null;
};

const portableExe = flag("portable");
const installedExe = flag("installed");
if (!portableExe) {
  console.error("usage: verify-portable-runtime.mjs --portable <exe> [--installed <exe>]");
  process.exit(1);
}

const windows = process.platform === "win32";
const exeName = windows ? "ArcScan.exe" : "ArcScan";
// How long to let a launch run before ending it. The app has nothing to do here
// beyond starting, opening its database and mounting the interface.
const RUN_MS = Number(process.env.ARCSCAN_VERIFY_RUN_MS || 15000);

const root = path.join(tmpdir(), `arcscan-portable-verify-${process.pid}`);
rmSync(root, { recursive: true, force: true });
mkdirSync(root, { recursive: true });

let failures = 0;
function check(ok, description, detail) {
  if (ok) console.log(`  ok    ${description}`);
  else {
    failures += 1;
    console.log(`  FAIL  ${description}${detail ? `\n        ${detail}` : ""}`);
  }
}

/** Is there a graphical session to launch a window into? */
const displayAvailable = () =>
  windows ||
  Boolean(process.env.DISPLAY || process.env.WAYLAND_DISPLAY) ||
  spawnSync("which", ["xvfb-run"], { encoding: "utf8" }).status === 0;

/** Wrap a launch in xvfb-run where there is no display of our own. */
const launchCommand = (exe) =>
  windows || process.env.DISPLAY || process.env.WAYLAND_DISPLAY
    ? [exe, []]
    : ["xvfb-run", ["-a", exe]];

/**
 * End a launch and everything it started.
 *
 * Killing the process that was spawned is not enough. Under xvfb-run that
 * process is a shell, and ArcScan is its child; kill only the shell and ArcScan
 * keeps running -- still holding the same-folder lock, so the next launch in
 * this script is refused and the check that follows reports a failure that is
 * really this script's fault. So the whole process group goes.
 */
function killTree(child) {
  if (!child.pid) return;
  if (windows) {
    spawnSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });
    return;
  }
  try {
    process.kill(-child.pid, "SIGKILL");
  } catch {
    try {
      child.kill("SIGKILL");
    } catch {
      // Already gone, which is the outcome being asked for.
    }
  }
}

/** Long enough for the operating system to release a lock the kernel holds. */
const settle = () => new Promise((r) => setTimeout(r, 1500));

/**
 * Run an ArcScan copy from an unrelated working directory and stop it again.
 *
 * The working directory is deliberately somewhere else: a portable root taken
 * from the working directory rather than from the executable would pass every
 * other check here and fail the moment a shortcut launched it.
 */
function run(folder) {
  const [command, prefix] = launchCommand(path.join(folder, exeName));
  return new Promise((resolve) => {
    const child = spawn(command, prefix, {
      cwd: tmpdir(),
      stdio: ["ignore", "pipe", "pipe"],
      // Its own process group, so killTree can end the whole launch.
      detached: !windows,
    });
    let out = "";
    child.stdout.on("data", (d) => (out += d));
    child.stderr.on("data", (d) => (out += d));

    let settled = false;
    const finish = (code) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve({ code, output: out, ranToTimeout: code === null });
    };
    // Still running after the window should have opened is success for a GUI
    // application, so end it and report as much.
    const timer = setTimeout(() => {
      killTree(child);
      finish(null);
    }, RUN_MS);
    child.on("exit", (code) => finish(code ?? 0));
    child.on("error", (e) => {
      out += String(e);
      finish(1);
    });
  });
}

/**
 * Launch a copy that is expected to be refused, and wait for it to say so.
 *
 * On Linux the process exits, so watching for the exit is enough. On Windows the
 * refusal is a modal MessageBoxW and the process sits there until somebody
 * presses OK, which no CI runner is going to do -- so what is watched for is the
 * message on standard error, which ArcScan writes before raising the dialog. The
 * launch is then ended either way.
 */
function runExpectingRefusal(folder, pattern) {
  const [command, prefix] = launchCommand(path.join(folder, exeName));
  return new Promise((resolve) => {
    const child = spawn(command, prefix, {
      cwd: tmpdir(),
      stdio: ["ignore", "pipe", "pipe"],
      detached: !windows,
    });
    let out = "";
    let settled = false;
    const finish = (refused) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      killTree(child);
      resolve({ refused, output: out });
    };
    const watch = (d) => {
      out += d;
      if (pattern.test(out)) finish(true);
    };
    child.stdout.on("data", watch);
    child.stderr.on("data", watch);
    child.on("exit", (code) => finish(code !== 0 && code !== null));
    child.on("error", () => finish(false));
    // Long enough for a refusal, which happens before Tauri starts at all.
    const timer = setTimeout(() => finish(false), Math.min(RUN_MS, 12000));
  });
}

function stage(name, exe) {
  const folder = path.join(root, name);
  mkdirSync(folder, { recursive: true });
  copyFileSync(exe, path.join(folder, exeName));
  if (!windows) spawnSync("chmod", ["+x", path.join(folder, exeName)]);
  return folder;
}

const listing = (dir) => {
  try {
    return readdirSync(dir).sort();
  } catch {
    return null;
  }
};

/** Where the installed edition keeps its data, per platform. */
function installedDataDir() {
  const id = "com.arcscan.app";
  if (windows) return path.join(process.env.APPDATA || "", id);
  if (process.platform === "darwin") {
    return path.join(homedir(), "Library", "Application Support", id);
  }
  return path.join(process.env.XDG_DATA_HOME || path.join(homedir(), ".local", "share"), id);
}

function python(source, ...argv) {
  const result = spawnSync("python3", ["-c", source, ...argv], { encoding: "utf8" });
  return result.status === 0 ? result.stdout : null;
}

/**
 * ArcScan's localStorage keys, read out of a WebKitGTK profile.
 *
 * Only possible on the WebKitGTK backend, where localStorage is a SQLite file.
 * WebView2 uses LevelDB, which would need a dependency to read, so on Windows
 * this returns null and the caller checks the profile directory rather than its
 * contents. The mechanism being verified -- that the profile directory is the
 * portable one -- is the same either way.
 */
function localStorageKeys(profileDir) {
  const dir = path.join(profileDir, "localstorage");
  const store = (listing(dir) || []).find((f) => f.endsWith(".localstorage"));
  if (!store) return null;
  const out = python(
    [
      "import sqlite3,sys,json",
      "c=sqlite3.connect(sys.argv[1])",
      "d={k:(v.decode('utf-16-le') if isinstance(v,bytes) else v) for k,v in c.execute('select key,value from ItemTable')}",
      "print(json.dumps(d))",
    ].join("\n"),
    path.join(dir, store),
  );
  if (!out) return null;
  try {
    return JSON.parse(out);
  } catch {
    return null;
  }
}

function seedPreferences(profileDir, settings, recents) {
  const dir = path.join(profileDir, "localstorage");
  const store = (listing(dir) || []).find((f) => f.endsWith(".localstorage"));
  if (!store) return false;
  return (
    python(
      [
        "import sqlite3,sys",
        "c=sqlite3.connect(sys.argv[1])",
        "enc=lambda s: s.encode('utf-16-le')",
        "c.execute('insert or replace into ItemTable(key,value) values(?,?)',('arcscan-settings',enc(sys.argv[2])))",
        "c.execute('insert or replace into ItemTable(key,value) values(?,?)',('arcscan-recent-targets',enc(sys.argv[3])))",
        "c.commit(); c.close()",
      ].join("\n"),
      path.join(dir, store),
      JSON.stringify(settings),
      JSON.stringify(recents),
    ) !== null
  );
}

/**
 * Does a refusal suggest putting the data somewhere other than where the
 * operator chose?
 *
 * The technical detail line under each message names the path involved, and on
 * Windows a staging folder under `%TEMP%` legitimately contains the literal
 * string "AppData" -- `C:\Users\RUNNER~1\AppData\Local\Temp\...`. An earlier
 * version of this check tested the whole output for that word and failed a CI
 * run over a path it had itself created, which is the test being wrong rather
 * than ArcScan.
 *
 * So the detail lines are dropped and the operator-facing sentences are what is
 * examined, which is where an offer would actually be made. The stronger form
 * of this property -- that portable ArcScan never *writes* to the
 * application-data directory -- is checked separately in step 5, against the
 * filesystem rather than against wording.
 */
const DETAIL_PREFIXES = [
  /^Data folder:/,
  /^Location:/,
  /^Could not (create|write to|lock|open) /,
];

function offersAnywhereElse(output) {
  const prose = output
    .split(/\r?\n/)
    .filter((line) => !DETAIL_PREFIXES.some((prefix) => prefix.test(line.trim())))
    .join("\n");
  return /appdata|application data|instead|fell back|falling back/i.test(prose);
}

if (!displayAvailable()) {
  console.log("No display and no xvfb-run: cannot launch a window. Skipping.");
  process.exit(0);
}

console.log(`Portable runtime verification (${process.platform})`);
console.log(`  working directory during every launch: ${tmpdir()}`);

const a = stage("A", portableExe);
const b = stage("B", portableExe);
const dataA = path.join(a, "ArcScanData");
const dataB = path.join(b, "ArcScanData");
const profileA = path.join(dataA, "WebView");
const installedDir = installedDataDir();
const installedBefore = listing(installedDir);

console.log("\n1. First launch of portable folder A");
const firstA = await run(a);
await settle();
check(firstA.ranToTimeout, "it starts and stays running", firstA.output.slice(-400));
check(existsSync(dataA), "ArcScanData is created beside the executable");
check(existsSync(path.join(dataA, "arcscan.db")), "the database is inside ArcScanData");
check(existsSync(path.join(dataA, "runtime.lock")), "the same-folder lock is inside ArcScanData");
check(existsSync(profileA), "the WebView profile is inside ArcScanData");
check(
  (listing(profileA) || []).length > 0,
  "the WebView profile directory has been written to",
  `contents: ${JSON.stringify(listing(profileA))}`,
);
check(
  !existsSync(path.join(a, "arcscan.db")),
  "nothing is written loose beside the executable",
  `beside the exe: ${JSON.stringify(listing(a))}`,
);

console.log("\n2. A second copy from the same folder");
const [heldCommand, heldPrefix] = launchCommand(path.join(a, exeName));
const held = spawn(heldCommand, heldPrefix, {
  cwd: tmpdir(),
  stdio: "ignore",
  detached: !windows,
});
await new Promise((r) => setTimeout(r, Math.min(RUN_MS, 8000)));
const secondA = await runExpectingRefusal(a, /already running from this folder/i);
check(secondA.refused, "it is refused, and says which folder is already in use", secondA.output.slice(-400));
check(
  !offersAnywhereElse(secondA.output),
  "and does not offer to put the data somewhere else",
  secondA.output.slice(-400),
);

console.log("\n3. A different portable folder, while A is still running");
const firstB = await run(b);
await settle();
check(firstB.ranToTimeout, "it starts", firstB.output.slice(-400));
check(existsSync(path.join(dataB, "arcscan.db")), "it has its own database");
if (!windows) {
  check(
    statSync(path.join(dataA, "arcscan.db")).ino !== statSync(path.join(dataB, "arcscan.db")).ino,
    "the two databases are different files",
  );
}
killTree(held);
await settle();

console.log("\n4. Preferences stay with the folder that holds them");
const keysBefore = localStorageKeys(profileA);
if (keysBefore === null) {
  console.log("  skip  localStorage contents are not readable on this webview backend");
  check(
    (listing(profileA) || []).length > 0 && (listing(path.join(dataB, "WebView")) || []).length > 0,
    "each folder has its own written-to WebView profile",
  );
} else {
  check("arcscan-theme" in keysBefore, "A's profile holds ArcScan's own preference keys");
  check(
    seedPreferences(profileA, { theme: "dark", density: "comfortable" }, ["10.42.7.0/24"]),
    "A's preferences can be set in A's own profile",
  );
  const relaunchA = await run(a);
  await settle();
  check(relaunchA.ranToTimeout, "A starts again from the same folder once the first copy is closed");
  const keysA = localStorageKeys(profileA) || {};
  check(keysA["arcscan-theme"] === "dark", "A reads them back after a relaunch", JSON.stringify(keysA));
  check(
    keysA["arcscan-recent-targets"] === '["10.42.7.0/24"]',
    "A's recent targets survive the relaunch",
  );
  await run(b);
  await settle();
  const keysB = localStorageKeys(path.join(dataB, "WebView")) || {};
  check(keysB["arcscan-theme"] !== "dark", "B does not inherit A's theme", JSON.stringify(keysB));
  check(keysB["arcscan-recent-targets"] === undefined, "B does not inherit A's recent targets");
  check((localStorageKeys(profileA) || {})["arcscan-theme"] === "dark", "and B has not changed A's");
}

console.log("\n5. The installed application-data directory");
check(
  JSON.stringify(listing(installedDir)) === JSON.stringify(installedBefore),
  "portable ArcScan never wrote to it",
  `before: ${JSON.stringify(installedBefore)}\n        after:  ${JSON.stringify(listing(installedDir))}`,
);

if (installedExe) {
  console.log("\n6. The installed edition, on the same machine");
  const installed = stage("installed", installedExe);
  const runInstalled = await run(installed);
  await settle();
  check(runInstalled.ranToTimeout, "it starts", runInstalled.output.slice(-400));
  check(
    !existsSync(path.join(installed, "ArcScanData")),
    "it creates no ArcScanData beside itself",
    `beside the exe: ${JSON.stringify(listing(installed))}`,
  );
  check(
    existsSync(path.join(installedDir, "arcscan.db")),
    "its database is in the application-data directory",
    installedDir,
  );
  check(existsSync(path.join(dataA, "arcscan.db")), "it left portable folder A's data alone");
  const installedKeys = localStorageKeys(installedDir);
  if (installedKeys) {
    check(
      installedKeys["arcscan-theme"] !== "dark",
      "it did not inherit portable folder A's theme",
      JSON.stringify(installedKeys),
    );
  }
}

rmSync(root, { recursive: true, force: true });

console.log("");
if (failures > 0) {
  console.error(`${failures} expectation(s) failed.`);
  process.exit(1);
}
console.log("Every expectation held.");
