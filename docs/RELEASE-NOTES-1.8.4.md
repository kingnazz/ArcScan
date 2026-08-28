# ArcScan 1.8.4 release notes

ArcScan 1.8.4 adds disposable Windows Portable ZIPs for x64 and ARM64. Portable
is a field session, not a persistent USB installation: extract it, launch without
installing, scan and investigate, export anything worth keeping, then close it.
The next Portable launch starts fresh.

The scanner and discovery behavior are unchanged from 1.8.3. Installed ArcScan,
its updater and the macOS universal build are also unchanged.

## The Portable workflow

1. Download the Portable ZIP that matches the Windows architecture.
2. Extract it to any suitable folder.
3. Run `ArcScan.exe`; no installer or administrator rights are required.
4. Use Inventory, Changes, History, discovery, device details and ports normally
   during the session.
5. Export CSV, JSON or XML for anything that should survive.
6. Close ArcScan and let it clean its temporary session.
7. Start from empty Portable state next time.

The extracted folder contains only `ArcScan.exe` and
`README-PORTABLE.txt`. ArcScan does not create a persistent database or WebView
profile beside the executable, and that folder does not need to be writable.

## Isolated temporary sessions

Every Portable process creates its own directory at:

```text
<system temp>/ArcScanPortable/sessions/<unique-session-id>/
```

That directory may contain the regular `arcscan.db`, a dedicated `WebView/`
profile and validated ownership metadata. The existing SQLite architecture is
used so normal ArcScan features continue to work within the session.

The WebView profile is selected before the window is created. Portable therefore
cannot inherit Installed ArcScan's theme, recent targets, scan defaults, column
layout or other WebView-backed preferences. Two Portable processes also receive
different profiles and databases, even if both were launched from the same
extracted copy.

Installed ArcScan continues to use its existing application-data database and
default WebView profile. Installed and Portable may run at the same time without
sharing scan state, names, notes, discovery evidence or preferences.

## Cleanup designed to refuse unsafe targets

On normal shutdown ArcScan closes SQLite, tears down the WebView and then removes
its owned session. A later Portable startup examines the sessions namespace and
cleans validated stale sessions left by a crash, power loss or failed shutdown.
Active sessions remain untouched.

Recursive cleanup requires all of the following before any payload is removed:

- an exact direct child of the fixed Portable sessions namespace;
- a compact lowercase UUID v4 with the RFC variant bits;
- a bounded ownership marker with the ArcScan product, marker kind, format,
  matching session identifier, timestamp and process identifier;
- an inactive operating-system session lock;
- only the known database, SQLite sidecars and link-free WebView tree.

Malformed, missing, oversized or mismatched markers are refused. Unknown files,
symlinks, reparse points, active locks and arbitrary temporary directories are
also refused. Cleanup removes the ownership marker last and preserves it for a
later retry whenever possible; any orphan whose ownership can no longer be
proved is refused rather than deleted.

## Exports intentionally survive

CSV, JSON and XML exports are the persistence mechanism. Choose a destination
outside the temporary session and the exported file survives session cleanup.
Portable refuses an export path inside its own session so a successful-looking
export cannot disappear when the app closes.

Portable does not intentionally retain its database, Inventory, History,
Changes, device names, notes, trust states, type corrections, discovery evidence,
recent targets or WebView-backed preferences between launches.

## Portable updates are manual

Portable does not contain the NSIS installer updater and cannot apply an
Installed ArcScan update. To update:

1. export anything you want to keep;
2. finish the session and close ArcScan;
3. download the latest Portable ZIP for the same architecture;
4. extract it and start a new session.

There is no Portable database or profile to carry forward. `latest.json` remains
an Installed-updater-only manifest, and the Installed updater continues to check,
verify, install and restart exactly as before.

## Release assets

Windows releases continue to include both editions for both architectures:

- Windows x64 Installer;
- Windows x64 Portable ZIP;
- Windows ARM64 Installer;
- Windows ARM64 Portable ZIP.

Portable packaging verifies the exact PE architecture and accepts only the two
minimal ZIP members. It excludes the installer, database, WebView profile,
updater manifest, source and debug files.

The website now presents Installer and Portable choices for Windows and explains
that Portable sessions are temporary. macOS remains the universal DMG with no
Portable choice.

## Storage-media behavior

The folder containing `ArcScan.exe` is not used as a data root. Portable can
therefore launch by design when that folder is read-only, provided Windows
temporary storage is available and writable. If a temporary session cannot be
created, ArcScan stops with a specific error and never falls back to Installed
AppData or the Installed WebView profile.

Launching from USB stores live SQLite and WebView state on the machine's local
temporary volume, not on removable media. The USB drive should still remain
connected until ArcScan exits because Windows or the runtime may need executable
resources. Real removal, read-only-media, network-policy and endpoint-security
behavior requires verification on representative Windows hardware.

## What did not change

- network scanning, ports, discovery and classification;
- Inventory, Changes, History and device-detail behavior within a session;
- the SQLite schema and migrations;
- Installed ArcScan's data location, WebView profile, updater and uninstaller;
- the Content Security Policy and privacy model;
- macOS packaging and behavior;
- the absence of telemetry, accounts and cloud storage.

For operational details, see [PORTABLE.md](PORTABLE.md). For the path audit,
marker format and cleanup invariants, see
[PORTABLE-ARCHITECTURE.md](PORTABLE-ARCHITECTURE.md).

## Known limitations

- Portable is Windows-only.
- WebView2 is still required and is not bundled in the ZIP.
- Portable state is deliberately not recoverable after successful cleanup; use
  an export before closing.
- A crash may leave an ownership-marked session until a later Portable startup
  can safely remove it.
- ArcScan is not currently signed with a paid Windows publisher certificate, so
  SmartScreen may warn on first launch.
- Final confidence in executable-folder permissions, USB removal, network-share
  launch, endpoint-security interaction and native ARM64 behavior requires real
  Windows runtime testing.
