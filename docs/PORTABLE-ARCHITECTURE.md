# Portable architecture and the persistence audit

ArcScan 1.8.4 adds a Windows **Portable edition**. This document is the audit
that had to come first — every place ArcScan persists or caches state, where
that state lives in each edition, and which of those decisions the portable
build changes — followed by the architecture built on top of it.

It is written to be checkable. Each claim below names the file and the call that
makes it true.

---

## 1. The audit

Searched for `app_data_dir`, `app_local_data_dir`, `app_cache_dir`,
`app_config_dir`, `data_dir`, `localStorage`, `sessionStorage`, `IndexedDB`,
`WebView`, `webview`, `preferences`, `settings`, `recent targets`, `theme`,
`columns`, `database`, `arcscan.db`, `updater`, `cache`, `temp` and `log` across
`src-tauri/src`, `src`, `public`, `scripts` and the workflows.

What that turns up is a *small* persistence surface, which is why this release
is tractable at all: ArcScan owns exactly two persistent stores.

### 1.1 SQLite database — ArcScan-owned, must persist, must be isolated

| | |
| --- | --- |
| Class | SQLite database |
| Written by | `src-tauri/src/db.rs`, opened once in `src-tauri/src/lib.rs` |
| Installed location | `app_data_dir()/arcscan.db` — `%APPDATA%\com.arcscan.app\arcscan.db` on Windows, `~/Library/Application Support/com.arcscan.app/arcscan.db` on macOS |
| Portable location | `<portable root>/ArcScanData/arcscan.db` |
| Must persist? | **Yes.** Scan history, the device inventory, names, notes, statuses, type overrides, discovery evidence, change events and network scopes all live here. |
| May be temporary? | No. |
| Must remain isolated? | **Yes.** Two editions sharing one database would mean one machine's inventory answering for a technician's USB stick. |

Before this release there was exactly one production call to
`app_data_dir()` in the whole codebase (`lib.rs`), and it produced this path.
That single call site is what made a clean portable split possible.

SQLite's own sidecar files (`arcscan.db-journal`, and `-wal`/`-shm` if a future
build enables WAL) are created by SQLite beside the database file, so they
follow the database into `ArcScanData` with no extra work.

### 1.2 WebView-backed preferences — ArcScan-owned, must persist, must be isolated

| | |
| --- | --- |
| Class | WebView/profile persistence (`localStorage`) |
| Written by | `src/lib/prefs.ts`, `src/hooks/useTheme.ts`, `public/theme-init.js` |
| Keys | `arcscan-settings`, `arcscan-recent-targets`, `arcscan-theme`, `arcscan-labels-imported`, and the read-once v1.6 legacy key `arcscan-known-devices` |
| Installed location | Whatever the WebView2 runtime chooses — see below |
| Portable location | `<portable root>/ArcScanData/WebView/` |
| Must persist? | **Yes.** Theme, default profile, port spec, timeouts, concurrency, density, hidden table columns, optional Inventory columns, sort key and direction, history retention, the Public IP switch, the update-check switch, the discovery master switch, the description-reading switch, reduced motion and the first-run flag. |
| May be temporary? | No. Losing this silently resets every preference. |
| Must remain isolated? | **Yes, and this is the release-blocking half.** Isolating only SQLite would ship a "portable" build whose theme, recent targets, column layout and discovery settings came from the installed copy. |

`arcscan-settings` is the whole `Settings` interface in `src/lib/prefs.ts`;
`arcscan-recent-targets` is the recent-target list; `arcscan-theme` is mirrored
separately so the inline script in `index.html` can paint the correct theme
before React mounts.

**What ArcScan controlled before this release: nothing.** Tauri's
`WebviewAttributes::data_directory` defaults to `None`
(`tauri-runtime-2.11.3/src/webview.rs`), that `None` is forwarded to
`WryWebContext::new` (`tauri-runtime-wry-2.11.4/src/lib.rs`) and on to
`CreateCoreWebView2EnvironmentWithOptions` (`wry-0.55.1/src/webview2/mod.rs`),
which means the WebView2 runtime applies **its own** default user-data folder.
ArcScan neither chose that path nor could predict it, and the runtime is
documented to fall back to the user's local app data when its first choice is
not writable. A portable build that left this alone would have a silent AppData
fallback built into it by the platform.

So 1.8.4 sets the directory explicitly for portable builds. Installed builds
keep passing nothing, exactly as every previous release did.

### 1.3 Updater state — not ArcScan-owned, temporary

| | |
| --- | --- |
| Class | Updater state |
| Written by | `tauri-plugin-updater`, driven from `src/hooks/useUpdater.ts` |
| Installed location | The plugin's download temp file, then the NSIS installer |
| Portable location | **Nothing is written.** The portable build does not link the updater plugin at all (see §6) |
| Must persist? | No |
| May be temporary? | Yes |
| Must remain isolated? | Moot in portable mode: there is nothing to isolate |

ArcScan stores no updater state of its own. The *preference*
(`settings.checkForUpdates`) is part of §1.2 and travels with the portable
folder like every other preference.

### 1.4 Cache and temp — not ArcScan-owned, temporary

| | |
| --- | --- |
| Class | Cache/temp |
| Written by | The WebView (HTTP cache, code cache), and `std::env::temp_dir()` in `db.rs` **tests only** |
| Installed location | Inside the WebView2 user-data folder; the system temp directory for tests |
| Portable location | Inside `ArcScanData/WebView/` — the same explicit directory as §1.2 |
| Must persist? | No |
| May be temporary? | Yes |
| Must remain isolated? | Yes, and it is, for free: it lives inside the profile directory §1.2 already isolates |

There is no ArcScan cache, no ArcScan log file and no ArcScan temp file in
production code. `grep` for `println!`/`eprintln!` across `src-tauri/src` finds
one hit, in a test.

### 1.5 Explicit user exports — user-owned, outside both editions

| | |
| --- | --- |
| Class | Explicit user export |
| Written by | `commands::save_text`, after the native save dialog |
| Installed location | Wherever the operator pointed the dialog |
| Portable location | Wherever the operator pointed the dialog — **not** forced into `ArcScanData` |
| Must persist? | The operator decides |
| May be temporary? | The operator decides |
| Must remain isolated? | N/A — it left ArcScan |

`save_text` validates that the path is absolute and ends in `.csv`, `.json` or
`.xml`, and caps the size. Portable mode changes none of that.

### 1.6 Operating-system integration — installer-owned

| | |
| --- | --- |
| Class | OS integration |
| Written by | The NSIS installer (per-machine), and the OS itself |
| Installed location | Uninstall registry entries, Start-menu and desktop shortcuts, the install directory |
| Portable location | **None.** No installer runs, no shortcut is created, no registry key is written for ArcScan state |
| Must persist? | Installed: yes, that is what an installer is for |
| May be temporary? | N/A |
| Must remain isolated? | Yes: a portable copy must not register itself on the machine |

Windows itself still keeps ordinary runtime records that no application
controls — MUICache, jump-list and prefetch entries, WebView2's own registry
presence. ArcScan does not claim otherwise anywhere in its documentation or on
its website, and §9 says so out loud.

### 1.7 Summary table

| Item | Class | Installed | Portable | Persist | Temporary | Isolated |
| --- | --- | --- | --- | --- | --- | --- |
| `arcscan.db` | SQLite | `app_data_dir()` | `ArcScanData/` | Yes | No | Yes |
| `localStorage` keys | WebView profile | WebView2 default | `ArcScanData/WebView/` | Yes | No | Yes |
| WebView cache | Cache | WebView2 default | `ArcScanData/WebView/` | No | Yes | Yes |
| Updater download | Updater | Plugin temp | Not present | No | Yes | N/A |
| Exports | User export | User's choice | User's choice | User's choice | User's choice | N/A |
| Shortcuts, uninstall keys | OS integration | Installer | None | Yes | No | Yes |
| `runtime.lock` | Portable runtime | Not present | `ArcScanData/` | No | Yes | Per data root |

---

## 2. Build-time edition identity

Portable mode is a property of the **binary**, decided at compile time, and
nothing observable at runtime can change it.

* `src-tauri/Cargo.toml` declares a `portable` feature.
* `src-tauri/src/runtime.rs` reads it once, through `cfg!(feature = "portable")`,
  into a single `Edition` constant.
* Nothing else in the codebase branches on anything else to decide the edition.

This is deliberately *not* "is there an `ArcScanData` folder next to me". A
marker-file switch means an installed ArcScan turns portable because a user
created a folder with an unlucky name, and a portable ArcScan turns installed
because the folder was deleted. Both are silent data-location changes, which is
the one thing a persistence design must never do.

Consequences, each covered by a test in `runtime.rs`:

* the installed binary reports installed, always;
* the portable binary reports portable, always;
* renaming or moving the executable does not change the edition;
* creating or deleting `ArcScanData` does not change the edition;
* an installed executable cannot become portable, and the reverse;
* the frontend is *told* the edition by the backend and can only display it.

## 3. `RuntimeInfo` and `RuntimePaths`

`runtime.rs` owns every app-owned path decision. Two types:

`RuntimePaths` is the internal resolution — `data_root`, `database_path`,
`webview_data_path`, `portable_root` — produced once during startup and used to
open the database and configure the WebView.

`RuntimeInfo` is what the frontend may see, over the `runtime_info` command:
`edition`, `version`, `architecture`, `data_root` (as a display string),
`writable` and `updater_mode`. It carries no path the frontend could use to
reconstruct another one, and the frontend never builds a portable path itself.

## 4. Portable root and layout

```
portable_root  = parent(current_exe())
data_root      = portable_root / ArcScanData
database       = data_root / arcscan.db
webview_profile= data_root / WebView
lock           = data_root / runtime.lock
```

`current_exe()` — never the current working directory, which a shortcut, a
`cmd /k`, an Explorer double-click and a scheduled task all set differently, and
which a user can change under a running process.

On disk:

```
ArcScan Portable/
├── ArcScan.exe
├── README-PORTABLE.txt
└── ArcScanData/            created on first successful launch
    ├── arcscan.db
    ├── WebView/
    └── runtime.lock
```

`ArcScanData` is created on first launch, not shipped in the ZIP, so the ZIP
never carries a pre-created user database.

## 5. Startup preflight

Portable startup refuses to open application state it has not proven it can
keep. In order:

1. resolve `current_exe()` and its parent;
2. classify the location, and refuse a network path (§7);
3. create `ArcScanData` if absent;
4. write a probe file into it, flush it, and delete it — an actual write, not a
   read-only flag on the metadata, because a full disk, a per-directory ACL and
   removable media that has gone away all report writable right up until the
   write;
5. acquire the exclusive same-root lock (§8);
6. open the database.

Any failure shows a portable-specific error and stops. There is **no** AppData
fallback, no second database, and no startup that looks successful with its data
going somewhere the operator did not choose.

## 6. Updater

Two independent layers, and they agree:

* **Backend.** The `portable` feature does not link `tauri-plugin-updater` or
  `tauri-plugin-process`. The installer-apply path is not hidden in a portable
  build; it is not compiled into it, so there is no NSIS updater to invoke and
  no restart-into-installer to trigger.
* **Frontend.** `runtime_info` reports `updater_mode: "manual"`, and the UI
  offers manual portable instructions and a link to the downloads page instead
  of an install action.

`latest.json` continues to describe installed, updater-signed artifacts only.
The portable ZIP is never an updater payload.

## 7. Network locations

SQLite over SMB is a documented hazard: advisory locking over a network
filesystem is unreliable, and a WebView profile is worse. 1.8.4 **refuses** to
run portable ArcScan from a network location rather than accept a corruption
risk it cannot mitigate.

Detection is Windows-specific and deliberately narrow: a UNC path
(`\\server\share`, including the `\\?\UNC\` form), or a drive letter that
`GetDriveTypeW` reports as `DRIVE_REMOTE`. Everything else — fixed disks,
removable media, RAM disks — is allowed.

## 8. Same-root locking

`ArcScanData/runtime.lock` is held with a real OS advisory lock for the life of
the process (`std::fs::File::try_lock`, which is `LockFileEx` with
`LOCKFILE_FAIL_IMMEDIATELY` on Windows and `flock`/`fcntl` elsewhere), not with
a "does the file exist" test.

That distinction is the whole point. A file-existence lock survives a crash and
bricks every future launch until someone deletes a file they have never heard
of. An OS lock is released by the kernel when the process ends, however it ends,
so a crashed portable ArcScan is launchable again immediately — the stale
`runtime.lock` file is simply relocked.

A second launch from the *same* folder is refused with a clear message. A launch
from a *different* portable folder is allowed. Installed ArcScan is unaffected:
it takes no lock, because it has no same-root problem to solve.

## 9. Honest limits

* Portable is Windows-only in 1.8.4. There is no portable macOS build, and the
  website does not claim one.
* The Microsoft Edge WebView2 Runtime is required. The portable ZIP does not
  ship a bootstrapper and portable ArcScan never installs system software, so
  "zero dependencies" is not a claim ArcScan makes.
* Portable self-update is not implemented. Updating is manual and documented.
* Removing a USB drive while ArcScan is running will make persistent writes
  fail. They are reported as failures; nothing is written elsewhere.
* Windows and the WebView2 runtime keep ordinary OS-level records outside the
  portable folder. The precise claim ArcScan makes is *"ArcScan-owned
  persistent data stays in `ArcScanData`"*, and nothing stronger.
* Installed and portable data never merge, in either direction, automatically.
