# ArcScan 1.8.4

**ArcScan can travel with you.**

v1.8.4 adds a Windows **Portable edition**, for x64 and for ARM64. Unzip it to a
folder or a USB drive and run it: no installer, no administrator rights, and
ArcScan's own data kept beside the application rather than in the machine's
application-data directory.

This is a distribution release. The scanner is the v1.8.3 scanner. No new
protocol, no new probe, no new outbound request, no new view, and no change to
how anything is discovered, identified, compared or stored.

The installed edition is untouched: same application-data location, same
database, same preferences, same migrations, same updater, same window. Install
over 1.8.x, 1.7.x or 1.6.x without losing anything.

Reference documentation: [PORTABLE.md](PORTABLE.md). Design and reasoning:
[PORTABLE-ARCHITECTURE.md](PORTABLE-ARCHITECTURE.md).

---

## What is new

### A portable build, meant literally

Plenty of tools call a ZIP portable while still writing their database into the
user's application data. Move the folder and the history stays behind; take it
to another machine and it starts empty.

ArcScan's portable edition is a **different build**, not the installer's
executable in a ZIP. It works out which folder it is running from, from the
executable itself, and keeps its data there:

```
ArcScan Portable\
├── ArcScan.exe
├── README-PORTABLE.txt
└── ArcScanData\               created on the first successful launch
    ├── arcscan.db             scan history, inventory, names, notes, statuses
    ├── WebView\               theme, preferences, recent targets, columns
    └── runtime.lock           held while ArcScan is running
```

**Both stores travel, not just the database.** The `WebView` directory is the
half that is easy to forget and just as important: theme, default profile, port
specification, timeouts, concurrency, row density, hidden table columns,
optional Inventory columns, sort order, history retention, the Public IP switch,
the update-check switch, the discovery switches, reduced motion and the
first-run flag. A build that isolated only the database would be a portable
ArcScan whose preferences came from an installed copy.

**Said precisely:** *ArcScan-owned persistent data stays in `ArcScanData`.* That
is the claim in full. Windows itself, and the WebView2 runtime, keep ordinary
records of programs that have run on a machine, and no application controls
those. ArcScan does not write its database, its preferences or its cache
anywhere except the folder above.

### The edition is decided when the binary is built

Not by what is in the folder. A marker file or an `ArcScanData` directory as the
switch would mean an installed ArcScan reading a different database because
somebody created a folder with an unlucky name, and a portable ArcScan reading
the installed one because the folder was deleted or quarantined. Both are silent
data-location changes, which is the one thing a persistence design must never
make.

So the installed binary is installed whatever surrounds it, the portable binary
is portable, renaming or moving the executable changes nothing, and creating or
deleting `ArcScanData` changes nothing.

### Settings says which edition, and where

```
ArcScan 1.8.4
Portable edition · Windows x64

Data location
E:\Tools\ArcScan\ArcScanData
```

With `Copy data path`, and `Open data folder` in the desktop app. The
architecture comes from the build rather than from the machine, so an ARM64 copy
says ARM64 and an x64 copy running on an ARM64 machine says x64.

For an installed copy this is deliberately understated: its data has always
lived somewhere nobody had to think about, and it still does.

### Installed and portable are independent

Both can be on one computer and in use at the same time. Neither reads the
other's history, names, notes, statuses or settings, and two portable folders
are equally independent of each other.

**Nothing is merged, in either direction.** A portable copy starts with its own
empty history rather than importing the installed one, and the installed app
never goes looking for a portable folder. Moving data between them is a
deliberate act: close both, copy `arcscan.db`.

### It refuses rather than falling back

Portable ArcScan proves it can keep its data before it opens any, and says so
clearly when it cannot. **There is no fallback to the application-data
directory in any of these cases.**

**A folder it cannot write to.** Checked by creating a file, writing to it,
flushing it to the device and deleting it again. Reading a read-only attribute
would be cheaper and would be wrong: a directory permission, a full disk, a
write-protect switch and media that has already been pulled all report a
writable folder right up until something is written to it.

> ArcScan Portable cannot save data in this folder.
>
> Move the ArcScan Portable folder to a writable local folder or USB drive and
> try again.

**A network location.** A UNC path, or a drive letter Windows reports as a
network drive. SQLite's advisory locking over SMB is unreliable and a WebView
profile over a share is worse, so this is refused rather than risked.

> ArcScan Portable is running from a network location.
>
> Copy the ArcScan Portable folder to a local or removable drive and run it
> again.

**A second copy from the same folder.** Two ArcScans sharing one `ArcScanData`
would corrupt it.

> ArcScan Portable is already running from this folder.
>
> Close the other ArcScan window before starting another copy from the same
> portable folder.

The lock behind that last one is a real operating-system lock held for the life
of the process, not a "does this file exist" test. A crash therefore costs
nothing: the kernel releases the lock however the process ended, and the next
launch relocks the file that was left behind. A file-existence lock would have
blocked every future launch until somebody found and deleted a file they had
never heard of.

### A drive that disappears fails honestly

If a USB drive is removed while ArcScan is running, database writes start
failing, and ArcScan reports them as failures. A rename, a note, a status change
or a saved scan that could not be written is reported as not written; ArcScan
never says a save succeeded when it did not. Scan results already in memory stay
on screen where they can. No second database appears anywhere, nothing falls
back, and no automatic repair is attempted.

### Updating a portable copy

Portable ArcScan still tells you when a newer version exists, because an
out-of-date network tool is a problem in itself. It does not install it, and the
portable build **does not contain the installer updater at all** rather than
merely hiding its button.

1. Close ArcScan.
2. Download the newer Windows Portable ZIP for your architecture.
3. Extract it somewhere separate.
4. Copy `ArcScan.exe` and `README-PORTABLE.txt` over the old ones.
5. Leave `ArcScanData` exactly where it is.
6. Launch ArcScan. It upgrades its own database if the new version needs to.

## Release assets

```
ArcScan_1.8.4_x64-setup.exe                    Windows x64 installer
ArcScan_1.8.4_arm64-setup.exe                  Windows ARM64 installer
ArcScan_1.8.4_universal.dmg                    macOS universal
ArcScan_1.8.4_windows-x64-portable.zip         Windows x64 portable
ArcScan_1.8.4_windows-arm64-portable.zip       Windows ARM64 portable
```

Each portable ZIP contains exactly `ArcScan.exe` and `README-PORTABLE.txt`. No
installer, no updater manifest, no signature, no debug symbols, no source, and
no pre-created database. The architecture inside each one is verified from its
PE header at build time, not from its filename.

`latest.json`, which the installed updater reads, continues to name only
installed, signed, updater-applicable artifacts. A portable ZIP is never an
updater payload.

## What did not change

- **The installed edition.** Same `%APPDATA%\com.arcscan.app\arcscan.db`, same
  preferences, same migrations, same updater, same window, same uninstaller.
- **The scanner.** Same read-only ICMP and TCP discovery, same ARP read, same
  profiles, same concurrency, same timeouts.
- **Discovery.** Same mDNS and SSDP, same URL guard, same bounds, same
  classification, same evidence aging, same `Copy discovery details`.
- **Inventory, Changes, History and the device panel.** Unchanged.
- **The database schema.** Version 6, the same as v1.8.3, in both editions.
- **Privacy.** Local only. No account, no cloud service, no telemetry, no
  analytics. The same two optional outbound requests, both listed in Settings:
  the update check, and the public-IP lookup that never runs without a press.
- **macOS.** No portable build, and no change to the DMG or its updater
  artifacts.
- **The Content Security Policy.** Not broadened. Portable mode needed no new
  origin and no new script source.

## Known limitations

1. Portable is Windows only in 1.8.4.
2. The Microsoft Edge WebView2 Runtime is required, so "no dependencies" is not
   a claim ArcScan makes. If the runtime is absent, the failure happens before
   ArcScan has an interface to explain it in.
3. Portable self-update is not implemented.
4. Network-share execution is refused.
5. Removing a USB drive while ArcScan is running interrupts database writes.
6. Two portable instances from the same folder are blocked.
7. ArcScan is still not code-signed with a paid publisher certificate, so
   SmartScreen warns on first launch.
8. Windows and WebView2 keep OS-level runtime records outside the portable
   folder.
9. Installed and portable data never merge automatically.

## Upgrading

**Installed.** Run the installer over any 1.6.x, 1.7.x or 1.8.x version. Nothing
moves and nothing is rewritten.

**Portable, from an installed copy.** The portable edition starts empty by
design. To carry your existing history over, close both, and copy
`%APPDATA%\com.arcscan.app\arcscan.db` into the portable `ArcScanData` folder
yourself. Preferences do not transfer; set them again in the portable copy.

**Portable, from an earlier portable copy.** There is no earlier portable copy:
1.8.4 is the first release with one.
