ArcScan __VERSION__ - Portable Edition for Windows __ARCH__
==========================================================

A disposable field tool for one ArcScan session. No installer, setup or
administrator rights.


GETTING STARTED
---------------

1. Extract this ZIP to a folder.
2. Run ArcScan.exe.
3. Scan the network and review Inventory, Changes, History, discovery,
   devices and ports during this session.
4. Export CSV, JSON or XML for anything you want to keep.
5. Close ArcScan.

The next Portable launch starts fresh.

Windows may show an "unknown publisher" warning the first time. See
SIGNING below.


TEMPORARY SESSION
-----------------

Every Portable process creates its own private session:

  <system temp>\ArcScanPortable\sessions\<unique-session-id>\
    arcscan.db
    WebView\
    validated ArcScan ownership and active-lock files

The ordinary SQLite database keeps ArcScan features working while the
window is open. The dedicated WebView profile keeps Portable preferences
separate from Installed ArcScan.

On normal shutdown ArcScan closes the database and WebView, then removes
its owned session. A later Portable launch safely cleans stale ArcScan-
owned sessions left by a crash or failed shutdown. Cleanup requires the
exact session namespace, a valid matching ownership marker, an inactive
lock and only known ArcScan contents. It refuses arbitrary temporary
directories, unknown files, links and malformed markers.

Windows and WebView2 may keep ordinary operating-system records of a
program that ran. ArcScan's cleanup guarantee applies to the validated
session it owns, not to Windows-owned artifacts.


KEEPING AN EXPORT
-----------------

CSV, JSON and XML exports are the intentional persistence mechanism.
Choose a destination outside ArcScan's temporary session. The exported
file then belongs to you and survives session cleanup.

Portable refuses an export destination inside its temporary session so a
file cannot appear to save successfully and disappear when ArcScan closes.

Without an export, Portable does not intentionally retain the scan
database, Inventory, History, Changes, names, notes, discovery evidence,
recent targets or WebView-backed preferences between launches.


EXTRACTED FOLDER AND USB
------------------------

The folder containing ArcScan.exe does not need to be writable. Portable
does not create a database, profile or persistent data folder beside the
executable. A read-only extracted folder is supported when Windows system
temporary storage is available.

If ArcScan.exe is on a USB drive, leave the drive connected until ArcScan
closes. Live database and WebView state are in local system temporary
storage, but Windows or the runtime may still need executable resources.

If system temporary storage is unavailable or unwritable, Portable stops
with a clear error. It never falls back to Installed ArcScan AppData, the
Installed WebView profile or the executable folder.


CONCURRENT AND INSTALLED USE
----------------------------

Two Portable processes receive different temporary sessions and may run
at the same time, including two launched from the same extracted folder.

Installed ArcScan and Portable ArcScan are also completely separate and
may run together. Neither reads the other's database, Inventory, History,
names, notes, discovery evidence or preferences. Nothing is merged in
either direction.


UPDATING
--------

Portable ArcScan does not install updates and cannot apply the NSIS
Installer updater.

1. Export anything you want to keep.
2. Finish the session and close ArcScan.
3. Download the latest Portable ZIP for this architecture.
4. Extract it and run the new ArcScan.exe.

There is no Portable database or profile to copy forward. The new launch
starts fresh by design. Do not replace ArcScan.exe while it is running.


REMOVING
--------

Close every process launched from this copy, then delete the extracted
folder. Portable has no uninstaller, service, Start-menu entry or
scheduled task. Installed ArcScan is unaffected.


REQUIREMENTS
------------

* Windows 10 or later, __ARCH__.
* The Microsoft Edge WebView2 Runtime.

WebView2 ships with Windows 11 and current Windows 10 releases, so it is
usually already present. This ZIP does not install system software. If it
is missing, install "Microsoft Edge WebView2 Runtime" from Microsoft and
run ArcScan again.


SIGNING
-------

ArcScan is not code-signed with a paid publisher certificate, so Windows
SmartScreen may show an unknown-publisher warning. Choose "More info",
then "Run anyway". Download only from the official ArcScan release and
verify the published SHA-256 checksum.


PRIVACY
-------

ArcScan scans only what you point it at and does not upload scan data.
Portable has no automatic installer update path. The optional public-IP
lookup runs only when you press Check and can be disabled in Settings.

Full notes: https://kingnazz.github.io/ArcScan/privacy.html


MORE
----

Downloads and documentation: https://kingnazz.github.io/ArcScan/
Source and releases:         https://github.com/kingnazz/ArcScan

MIT licensed.
