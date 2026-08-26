# ArcScan Portable (Windows)

ArcScan 1.8.4 adds a Windows Portable edition. Unzip it to a folder or a USB
drive and run it: no installer, no administrator rights, and ArcScan's own data
kept beside the application rather than in the machine's application-data
directory.

This document is the reference. The short version ships inside the ZIP as
`README-PORTABLE.txt`; the design and the reasoning behind it are in
[PORTABLE-ARCHITECTURE.md](PORTABLE-ARCHITECTURE.md).

---

## Supported platforms

| | |
| --- | --- |
| Windows x64 | `ArcScan_1.8.4_windows-x64-portable.zip` |
| Windows ARM64 | `ArcScan_1.8.4_windows-arm64-portable.zip` |
| macOS | No portable build. Use the DMG. |
| Linux | No distribution of any kind. |

Windows 10 or later, and the Microsoft Edge WebView2 Runtime (see below).

The ARM64 ZIP contains a native ARM64 binary, verified from the PE header rather
than from the filename. Running the x64 build on Windows on ARM would work
through emulation but is slower; take the one that matches the machine.

## Folder layout

```
ArcScan Portable\
├── ArcScan.exe
├── README-PORTABLE.txt
└── ArcScanData\               created on the first successful launch
    ├── arcscan.db             scan history, inventory, names, notes, statuses
    ├── WebView\               theme, preferences, recent targets, columns
    └── runtime.lock           held while ArcScan is running
```

`ArcScanData` is not in the ZIP. It appears on the first launch, so no download
arrives carrying somebody else's starting state.

## Where the data lives

Settings shows the exact path, with `Copy data path` beside it and
`Open data folder` in the desktop app.

**Two stores, and both are portable.** The database is the obvious one. The
`WebView` directory is the one that is easy to forget and just as important: it
holds theme, default profile, port specification, timeouts, concurrency, row
density, hidden table columns, optional Inventory columns, sort order, history
retention, the Public IP switch, the update-check switch, the discovery switches,
reduced motion and the first-run flag. A build that isolated only the database
would be a portable ArcScan whose preferences came from an installed copy.

**Said precisely:** *ArcScan-owned persistent data stays in `ArcScanData`.* That
is the claim in full. Windows itself, and the WebView2 runtime, keep ordinary
records of programs that have run on a machine, and no application controls
those. ArcScan does not write its database, its preferences or its cache
anywhere except the folder above.

## Installed and portable together

Both editions can be installed on one computer and used at the same time.

| | Installed | Portable |
| --- | --- | --- |
| Database | `%APPDATA%\com.arcscan.app\arcscan.db` | `ArcScanData\arcscan.db` |
| Preferences | The WebView2 runtime's default location | `ArcScanData\WebView\` |
| Updates | Checked and installed in the app | Checked in the app, replaced by hand |
| Start menu | Yes | No |
| Uninstaller | Yes | Nothing to uninstall |
| Administrator rights | To install | Never |

Neither reads the other's history, names, notes, statuses or settings. Two
portable folders are equally independent of each other.

**Nothing is merged, in either direction.** A portable copy starts with its own
empty history rather than importing the installed one, and the installed app
never goes looking for a portable folder. Moving data between them is a
deliberate act: close both, and copy `arcscan.db` yourself.

## Updating

Portable ArcScan tells you when a newer version exists. It does not install it,
and the portable build does not contain the installer updater at all.

1. Close ArcScan.
2. Download the newer Windows Portable ZIP for your architecture.
3. Extract it somewhere separate.
4. Copy `ArcScan.exe` and `README-PORTABLE.txt` over the old ones.
5. Leave `ArcScanData` exactly where it is.
6. Launch ArcScan. It upgrades its own database if the new version needs to.

Do not copy over a running copy. Windows may refuse, and a half-replaced
application is worse than an old one.

The installed edition's updater is unchanged: it checks the signed feed,
verifies the signature, installs and restarts, exactly as before 1.8.4.

## Backing up

```
Close ArcScan.
Copy the entire ArcScan Portable folder.
```

That is the whole procedure, and it is also how you move a portable copy to
another drive or another computer. Nothing in the database depends on where it
was, so a copy works wherever it lands.

`ArcScanData` on its own is enough to preserve the history and the settings, if
you would rather back up only the data.

## Removing

```
Close ArcScan and delete the portable folder.
```

Nothing is left behind: no installer, no service, no ArcScan registry settings,
no Start-menu entry, no per-machine registration.

## Read-only and unwritable storage

Portable ArcScan proves it can write to the folder before it opens anything, by
creating a file, writing to it, flushing it to the device and deleting it again.
Reading a read-only attribute would be cheaper and would be wrong: a directory
permission, a full disk, a write-protect switch on an SD card, an antivirus
holding the folder and removable media that has already been pulled all report a
perfectly writable folder right up until something is written to it.

If that fails, ArcScan says so and stops:

```
ArcScan Portable cannot save data in this folder.

Move the ArcScan Portable folder to a writable local folder or USB drive
and try again.
```

**There is no fallback to the application-data directory.** A portable build
that quietly wrote somewhere else would be worse than one that refused, because
the operator would not find out until they went looking for the history.

## Network locations

Portable ArcScan **refuses** to run from a network location:

```
ArcScan Portable is running from a network location.

Copy the ArcScan Portable folder to a local or removable drive and run it
again.
```

That means a UNC path (`\\server\share`, including the `\\?\UNC\` spelling) or a
drive letter Windows reports as a network drive. Fixed disks, removable media and
RAM disks are all fine.

The reason is SQLite: advisory locking over SMB is unreliable, and a WebView
profile over a network share is worse. Refusing is a limitation, and it is a
deliberate one, chosen over a corruption risk that cannot be mitigated from
inside the application. If you need ArcScan on a file server, copy the folder to
local storage and run it there.

## WebView2

ArcScan uses the Microsoft Edge WebView2 Runtime for its interface, in both
editions. It ships with Windows 11 and with current Windows 10, so it is almost
certainly already present.

The portable ZIP deliberately does not include a bootstrapper for it, and
portable ArcScan never installs system software. If the runtime is missing,
install "Microsoft Edge WebView2 Runtime" from Microsoft and run ArcScan again.

Because WebView2 is required, **"no dependencies" is not a claim ArcScan makes.**

Honestly stated: if the runtime is absent, the failure happens before ArcScan has
an interface to explain it in, so what you see is a Windows-level error rather
than a friendly ArcScan message.

## Two copies from the same folder

Two ArcScans running out of one `ArcScanData` would corrupt it, so the second one
is refused:

```
ArcScan Portable is already running from this folder.

Close the other ArcScan window before starting another copy from the same
portable folder.
```

The lock is a real operating-system lock held for the life of the process, not a
"does this file exist" test. That distinction is what makes a crash harmless: the
kernel releases an OS lock when the process ends however it ends, so the next
launch simply relocks the `runtime.lock` file that was left behind. A
file-existence lock would have bricked every future launch until somebody found
and deleted a file they had never heard of.

A second copy from a *different* portable folder is allowed, and so is running
portable ArcScan alongside an installed one.

## A USB drive that disappears

If the drive is removed while ArcScan is running, writes to the database start
failing. ArcScan reports them as failures.

* A rename, a note, a status change or a saved scan that could not be written is
  reported as not written. ArcScan never says a save succeeded when it did not.
* Scan results already in memory stay on screen where they can.
* No second database is created anywhere else, and nothing falls back to the
  application-data directory.
* No automatic database repair is attempted: an interrupted write is not
  something to guess at.

Reconnect the drive and restart ArcScan. If the database was mid-write when the
drive went, SQLite's own journal recovers it on the next open.

## Signing

ArcScan is not code-signed with a paid publisher certificate, in either edition.
Windows SmartScreen shows an unknown-publisher warning on first launch: choose
**More info**, then **Run anyway**.

Update packages for the installed edition are signed with a minisign key and the
installed updater verifies that signature before installing. That protects the
update path, not the first download. Portable ZIPs are not updater payloads and
carry no minisign signature; each release asset has a SHA-256 digest on its
GitHub release page if you want to verify a download.

## Known limitations

1. Portable is Windows only in 1.8.4.
2. The WebView2 Runtime is required.
3. Portable self-update is not implemented; updating is manual.
4. Network-share execution is refused, not merely discouraged.
5. Removing a USB drive while ArcScan is running interrupts database writes.
6. Two portable instances from the same folder are blocked.
7. Signing status is unchanged from previous releases.
8. Windows and WebView2 keep OS-level runtime records outside the folder.
9. Installed and portable data never merge automatically.
