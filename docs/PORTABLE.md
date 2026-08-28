# ArcScan Portable (Windows)

ArcScan 1.8.4 adds a disposable Windows Portable edition. It is a field tool,
not a persistent installation:

1. Extract the ZIP.
2. Launch `ArcScan.exe` without installing it.
3. Scan the network.
4. Review Inventory, Changes, History, discovery, devices and ports during that
   session.
5. Export CSV, JSON or XML if anything should be retained.
6. Close ArcScan.
7. The next Portable launch starts fresh.

The short version ships inside the ZIP as `README-PORTABLE.txt`. The security
and persistence design is documented in
[PORTABLE-ARCHITECTURE.md](PORTABLE-ARCHITECTURE.md).

## Supported builds

| Platform | Portable asset |
| --- | --- |
| Windows x64 | `ArcScan_1.8.4_windows-x64-portable.zip` |
| Windows ARM64 | `ArcScan_1.8.4_windows-arm64-portable.zip` |
| macOS | No Portable build; use the unchanged universal DMG |
| Linux | No distribution |

Use the ZIP that matches the Windows architecture. Release packaging verifies
the executable's PE machine type rather than trusting the filename.

Each Portable ZIP contains exactly:

```text
ArcScan.exe
README-PORTABLE.txt
```

It contains no installer, database, WebView profile, updater manifest, debug
file or source file.

## A private temporary session

Every Portable process creates a unique session below the system temporary
directory:

```text
<system temp>/ArcScanPortable/
├── .sessions.lock
└── sessions/
    └── <unique-session-id>/
        ├── .arcscan-portable-session
        ├── .arcscan-portable-session.lock
        ├── arcscan.db
        └── WebView/
```

`arcscan.db` is the ordinary ArcScan SQLite database. It lets Inventory,
Changes, History, discovery evidence, device names and notes, network scopes and
ports work normally while the session is open. `WebView/` is a dedicated
profile for that process, so recent targets, theme and other WebView-backed
preferences cannot come from Installed ArcScan.

Neither store is intended to survive the Portable session. Closing Portable
ArcScan does not turn the extracted folder into a portable installation, and no
data folder is created beside `ArcScan.exe`.

## What survives

An explicit export is the persistence mechanism. Choose CSV, JSON or XML and
save it outside ArcScan's temporary session. That exported file belongs to you
and is not removed when the session is cleaned.

Portable ArcScan refuses an export destination inside its owned temporary
session. This prevents a successful-looking export from disappearing during
normal cleanup.

The following are session-only in Portable ArcScan:

- scan database and saved scan records;
- Inventory, History and Changes;
- device names, notes, trust states and type corrections;
- discovery evidence and recent targets;
- port, profile, timeout, concurrency, column and theme preferences;
- WebView cache and profile data.

If it matters after the window closes, export it first.

## Safe cleanup

On normal shutdown ArcScan closes SQLite, lets the WebView release its profile,
then removes the session it owns. If a crash, power loss or locked file prevents
that cleanup, a later Portable startup retries stale-session cleanup.

Cleanup is deliberately strict. ArcScan only removes a direct child of its
Portable sessions namespace when all of these are true:

- the directory name is a compact lowercase UUID v4 with the RFC variant bits;
- its ownership marker is valid and names the same session;
- it has the expected ArcScan product, marker kind and format;
- it is not an active session;
- the path and contents pass link, reparse-point and known-layout checks.

A missing, malformed, oversized or mismatched marker causes refusal. So do
unknown contents and paths outside the namespace. ArcScan never treats the
whole system temporary directory, an arbitrary folder or an unvalidated path as
a cleanup target.

Cleanup failure is safe rather than silent: the verified marker is kept for a
later retry whenever possible. If ownership can no longer be proven, the orphan
is refused instead of deleted. Windows and WebView2 may keep ordinary
operating-system records of programs that ran; ArcScan's cleanup guarantee
applies to its validated session, not to artifacts controlled by Windows.

## Concurrent and Installed use

Each Portable process receives a different session identifier, database and
WebView profile. Two Portable processes may run at the same time, including two
launched from the same extracted folder.

Installed ArcScan continues to use its existing application-data database and
default WebView profile. Portable does not read or write those locations, and
Installed ArcScan does not use the Portable sessions namespace. The two editions
may coexist and run simultaneously without sharing Inventory, History, Changes,
names, notes, discovery evidence or preferences.

There is no import or merge between editions. Export from the session when a
record should cross that boundary.

## Extracted-folder and USB behavior

The folder containing `ArcScan.exe` does not need to be writable. Portable does
not create a database or profile there, so extracting to read-only media is not
itself a reason for startup to fail.

A USB drive holds only the executable and packaged readme. The active database
and WebView profile stay in local system temporary storage, so they do not
disappear merely because the USB media is removed. Windows or the application
runtime may still need the original executable or its resources until the
process exits, so do not remove the drive while ArcScan is open. Real USB-removal,
read-only-media and network-share launch behavior remains subject to Windows,
WebView2 and endpoint-security policy and should be verified on representative
machines.

## Temporary-storage failure

Portable must create and lock its isolated temporary session before the app can
open. If Windows temporary storage is unavailable or unwritable, startup stops
with a Portable-specific error.

Portable never silently falls back to Installed ArcScan AppData, the Installed
WebView profile or the folder containing `ArcScan.exe`.

## Updating

Portable updates are manual. Portable does not include the NSIS installer
updater and cannot apply an Installed ArcScan update.

1. Export anything you want to keep.
2. Finish the current session and close ArcScan.
3. Download the latest Portable ZIP for the same architecture.
4. Extract it and launch the new `ArcScan.exe`.

There is no Portable database or profile to copy forward. The new launch starts
fresh by design.

The Installed edition's signed updater remains unchanged. `latest.json` remains
an Installed-updater manifest and never points to a Portable ZIP.

## WebView2 and signing

Both Windows editions require the Microsoft Edge WebView2 Runtime. It is already
present on current Windows 10 and Windows 11 systems. The Portable ZIP does not
install system software; if WebView2 is absent, install the runtime from
Microsoft before launching ArcScan.

ArcScan does not currently carry a paid Windows publisher certificate. Windows
SmartScreen may warn on first launch. Verify the release SHA-256 checksum and
download only from the official ArcScan release page.

## Removing Portable ArcScan

Close every process launched from that extracted copy, then delete the extracted
folder. There is no Portable uninstaller, service, Start-menu entry or scheduled
task. Any validated stale session left by an interrupted shutdown is eligible
for cleanup on a later Portable launch.

Installed ArcScan is removed through Windows Settings and is unaffected by
deleting a Portable ZIP or extracted folder.
