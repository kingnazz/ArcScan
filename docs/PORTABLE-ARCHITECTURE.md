# Portable architecture and persistence audit

ArcScan 1.8.4 introduces a disposable Windows Portable edition. Portable means
"run without installing for one field session," not "carry a persistent ArcScan
installation in a folder." This document records the state audit, isolation
boundaries and cleanup invariants behind that design.

The governing workflow is:

1. extract the ZIP;
2. launch the executable;
3. scan and investigate using the normal ArcScan features;
4. export CSV, JSON or XML when something should be retained;
5. close ArcScan;
6. start with fresh state on the next Portable launch.

## 1. Persistence audit

ArcScan has two application-owned stores that matter at runtime, plus explicit
user exports.

| State | Installed edition | Portable edition | Portable lifetime |
| --- | --- | --- | --- |
| SQLite database | Existing application-data `arcscan.db` | Unique session `arcscan.db` | Current process only |
| WebView preferences and cache | Existing default WebView profile | Unique session `WebView/` | Current process only |
| CSV, JSON and XML export | Operator-selected path | Operator-selected path outside the session | Retained by the operator |
| Updater payload | Installed updater temporary files | Updater not compiled in | Not applicable |
| Installer integration | Start menu, uninstall entry and install directory | None | Not applicable |

The SQLite database includes saved scans, Inventory, History, Changes, network
scopes, names, notes, trust state, type corrections and discovery evidence. The
WebView profile includes `localStorage` preferences such as theme, recent
targets, scan defaults, ports, timeouts, concurrency, table columns and optional
network-request switches. Isolating only one would leak state through the other,
so Portable isolates both.

Windows and WebView2 may keep ordinary operating-system records of an
application that ran. Those are outside ArcScan's ownership. The design does not
claim that running any Windows executable leaves no OS artifact; it claims that
ArcScan does not intentionally retain its scan database or WebView-backed state
between Portable launches.

## 2. Build-time edition identity

Edition identity is compiled into the Rust binary through mutually exclusive
Cargo features. It is not inferred from the executable name, current directory,
a neighboring file or a marker created by the operator.

Consequences:

- moving or renaming a Portable executable does not make it Installed;
- moving or renaming an Installed executable does not make it Portable;
- a missing session cannot trigger an Installed AppData fallback;
- no folder beside the executable can change the selected data location;
- the Portable build cannot include the Installed updater plugins.

The backend returns `edition`, `architecture`, `storage_mode` and `updater_mode`
to the frontend. Portable reports temporary storage and manual updates. It does
not expose the disposable session path as a reusable data root.

## 3. Per-process session layout

Portable startup uses `std::env::temp_dir()` as the system temporary root and
allocates a UUID v4 in compact lowercase form:

```text
<system temp>/ArcScanPortable/
├── .sessions.lock
└── sessions/
    ├── <session-a>/
    │   ├── .arcscan-portable-session
    │   ├── .arcscan-portable-session.lock
    │   ├── arcscan.db
    │   └── WebView/
    └── <session-b>/
        └── ...
```

The namespace lock serializes creation and cleanup decisions; it is not a
single-instance lock. Each process receives its own session directory and holds
only that session's active lock for its lifetime.

The executable directory is not part of path resolution. It does not have to be
writable and receives no database, profile, lock or persistent settings folder.

## 4. SQLite isolation

The same `Db::open` path and schema used by Installed ArcScan are used inside
the Portable session. This preserves normal behavior for scanning, Inventory,
Changes, History, discovery and device edits while the process is alive.

SQLite sidecars remain in the same session. Cleanup recognizes only the database
and its ordinary `-wal`, `-shm` and `-journal` forms. Installed ArcScan continues
to open its existing application-data database; Portable never asks Tauri for
that database path.

Proof obligations covered by runtime and command tests include:

- Portable and Installed database roots differ;
- two Portable layouts with different identifiers have different database paths;
- no executable-directory input participates in the Portable path;
- failure to create or open a temporary database is fatal, not a fallback;
- normal cleanup removes the owned database and SQLite sidecars;
- external CSV, JSON and XML exports remain after cleanup.

## 5. WebView/profile isolation

The WebView data directory has to be selected before WebView creation. ArcScan
therefore creates the main window programmatically from the same Tauri window
configuration and supplies `WebView/` from the Portable session only in the
Portable build.

Installed ArcScan continues to omit that override and uses its existing default
profile behavior. Portable cannot read Installed `localStorage`, and two
Portable processes cannot read each other's theme, recent targets, settings or
WebView cache.

The session path stays owned by Rust and is not offered as a place for the user
to save records. Windows WebView2 can retain profile handles until the owning
process has fully exited, so ArcScan starts the same Portable executable in a
private no-window monitor mode before the UI event loop. The monitor accepts no
path. It receives only the compact session identifier and the creator PID
already stored in the marker, reconstructs the fixed system-temp namespace,
revalidates marker and PID, and waits on the exact session's exclusive active
lock. Only after process exit releases that lock does it perform the same strict
cleanup with bounded deletion retries. Starting the monitor while the owner and
lock are known to be alive avoids late-shutdown and process-identifier reuse
races. A transient I/O failure while WebView2 is removing a volatile profile
entry restarts strict validation; an unsafe type, link, reparse point or unknown
entry still causes immediate refusal. The ZIP still contains only `ArcScan.exe`
and the Portable readme; there is no separate helper executable or persistent
service.

## 6. Ownership marker

Each session contains `.arcscan-portable-session`, a bounded JSON marker with
these exact fields:

- product (`ArcScan`);
- kind (`portable-session`);
- marker format version;
- the compact session identifier;
- an RFC 3339 creation timestamp;
- a nonzero creator process identifier.

Unknown fields are rejected. The marker is rejected if it is missing, larger
than the size limit, malformed, names another session, uses the wrong product,
kind or format, has an invalid timestamp, or has a zero process identifier.

The marker grants no authority by itself. The candidate must also be a plain
direct child of the exact `ArcScanPortable/sessions` namespace, have a valid
compact identifier, contain a plain active-lock file, and pass the payload
validation below.

## 7. Deletion boundary

Before removing any payload byte, cleanup proves all of the following:

1. the candidate's parent is exactly the sessions root;
2. its name is a compact lowercase UUID v4 with the RFC variant bits (32
   hexadecimal characters, version nibble `4`, variant nibble `8`, `9`, `a` or
   `b`);
3. the candidate is a real directory, not a symlink or reparse point;
4. the marker is a real file and validates for that same identifier;
5. the active-lock file is a real file and can be exclusively locked;
6. every top-level payload has an allowed ArcScan name and expected type;
7. the complete WebView tree contains no symlink, reparse point or unexpected
   file type;
8. the candidate is revalidated while the active lock is held.

Allowed top-level payloads are only `arcscan.db`, its known SQLite sidecars and
the `WebView/` directory. Unknown entries cause refusal rather than broader
deletion. The ownership marker is removed last, preserving proof for a later
retry if WebView2, antivirus or another Windows component temporarily holds a
file open. If the now-empty directory changes in the final removal race and the
best-effort marker restoration also fails, it remains unowned and future cleanup
refuses it instead of guessing.

There is no cleanup operation against the system temporary root, the whole
ArcScan namespace, an executable-relative directory, an Installed data path or a
path received from the frontend.

## 8. Startup and stale-session cleanup

Portable startup first validates or creates the fixed namespace, takes the
namespace lock and creates a new owned session. It then examines sibling session
directories for stale state.

- a locked session is active and remains untouched;
- a fully validated, unlocked session is stale and is removed;
- an invalid or unknown directory is ignored;
- an I/O failure is reported internally and retried on a later startup;
- stale cleanup failure does not deny the new independent session.

This makes a crash recoverable without weakening deletion authority. The kernel
releases the active file lock when the process ends, but the marker and session
remain available for a later ownership-checked cleanup pass.

## 9. Normal-shutdown cleanup

Portable uses Tauri's returning run loop so teardown order is explicit:

1. start the private no-window cleanup monitor before the UI event loop;
2. request scanner cancellation at application exit;
3. shut down the managed SQLite connection;
4. let Tauri destroy the window and WebView;
5. the monitor observes the exact active lock being released at process exit,
   then revalidates and removes the owned session;
6. return the original process exit code.

If the monitor cannot start or an inactive payload file cannot be removed after
the bounded retries, ArcScan leaves the validated marker in place and logs that
cleanup will be retried. It does not switch to an unvalidated recursive removal;
an orphan that cannot retain its marker is deliberately ineligible for later
cleanup.

## 10. Exports are the persistence boundary

Exports are built by the frontend and written through the existing validated
backend command. Paths must be absolute, use `.csv`, `.json` or `.xml`, and stay
within the existing export size limit.

Portable adds one boundary: the chosen destination may not be inside its
temporary session. The check covers direct lexical descendants and canonicalized
parent directories. An existing destination that is a link or Windows reparse
point is conservatively refused, including a dangling link whose future target
would be inside the session. A rejected destination receives instructions to
choose a location that will remain after ArcScan closes.

Cleanup tests create all three export formats outside a session, clean the
session and prove that every export remains unchanged.

## 11. Concurrency and coexistence

Two Portable processes may run simultaneously, even when launched from the same
extracted directory. They briefly serialize namespace maintenance, then hold
different session locks and use different SQLite and WebView paths.

Installed ArcScan may also run at the same time. Its database path, WebView
profile, updater capabilities and lifecycle are unchanged. No copying, merging
or lock-sharing occurs between editions.

## 12. Failure behavior and storage media

If the system temporary root cannot host a validated session, Portable shows a
specific fatal error and exits before creating the window or opening a database.
It never tries Installed AppData, the Installed WebView profile or the executable
directory as a substitute.

Because session state is local temporary data, a read-only executable directory
is supported by design. Launching from USB no longer places live SQLite or
WebView files on removable media. Removing the media while ArcScan is running is
still discouraged because Windows or the runtime may need executable resources.
Network-share launch behavior is likewise subject to Windows, WebView2 and
endpoint-security policy; it is no longer rejected to protect a database on the
share because no database is stored there.

These runtime cases require representative Windows x64, Windows ARM64,
read-only-media, USB and network-policy testing in addition to automated path
and cleanup tests.

## 13. Updater and release isolation

The Portable Cargo feature does not link `tauri-plugin-updater` or the process
restart plugin. The frontend presents a manual path:

1. export anything worth keeping;
2. finish the session and close ArcScan;
3. download the newest Portable ZIP for the same architecture;
4. extract it and begin a fresh session.

Portable cannot apply an NSIS updater and has no self-update mechanism.
`latest.json` continues to identify only the signed Installed updater assets.
The Installed updater's check, signature verification, install and restart path
is unchanged.

## 14. Packaging invariants

Release automation produces x64 and ARM64 Installer executables plus x64 and
ARM64 Portable ZIPs. A Portable ZIP must:

- contain exactly `ArcScan.exe` and `README-PORTABLE.txt`;
- contain the expected PE architecture, verified from the binary header;
- contain no installer, database, WebView profile, updater manifest, source,
  debug information or pre-created session;
- use the Portable feature without Installed updater features.

macOS remains one universal Installed DMG. No macOS Portable edition is offered
or implied.

## 15. Security invariants

The disposable architecture is acceptable only while all of these remain true:

- edition identity is build-time and mutually exclusive;
- every Portable process gets an unpredictable unique session;
- SQLite and WebView state share that session and nothing else;
- no failure path falls back to Installed or executable-relative storage;
- a valid ownership marker, exact namespace and inactive lock are all required
  before cleanup;
- links, reparse points, unknown payloads and malformed markers stop cleanup;
- normal cleanup begins only after SQLite and WebView teardown;
- stale cleanup preserves active sessions and ignores unowned paths;
- exports outside the session survive;
- Portable has no installer updater;
- Installed paths, updater and macOS behavior do not change.
