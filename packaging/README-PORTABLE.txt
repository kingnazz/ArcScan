ArcScan __VERSION__ - Portable Edition for Windows __ARCH__
==========================================================

A fast, private network inventory and scanner that runs from wherever you
put it. No installer, no setup, no administrator rights.


GETTING STARTED
---------------

1. Extract this ZIP to a folder first.

   Do not run ArcScan.exe from inside the ZIP. Windows opens archives in a
   temporary folder that it deletes without warning, so an ArcScan started
   from inside the ZIP would lose everything it saved.

2. Run ArcScan.exe.

3. A folder called ArcScanData appears beside it on the first launch.

   Windows may show an "unknown publisher" warning the first time. See
   SIGNING below.


WHERE YOUR DATA IS
------------------

ArcScan-owned persistent data stays in ArcScanData, beside ArcScan.exe:

  ArcScan Portable\
    ArcScan.exe
    README-PORTABLE.txt
    ArcScanData\
      arcscan.db      scan history, inventory, names, notes, statuses
      WebView\        theme, preferences, recent targets, column layout
      runtime.lock    held while ArcScan is running

Keep ArcScanData with ArcScan.exe and this copy of ArcScan keeps its
history and its settings. Move the whole folder -- to another drive, to a
USB stick, to another computer -- and everything comes with it.

Settings shows the exact path, with a button to copy it.

To be precise about the claim: this is about data ArcScan owns. Windows
itself, and the Microsoft Edge WebView2 Runtime, keep ordinary records of
programs that have run on a machine, and no application controls those.
ArcScan does not write its database, its preferences or its cache
anywhere except the folder above.


MOVING, BACKING UP AND REMOVING
-------------------------------

Move:    close ArcScan, move the whole portable folder.
Back up: close ArcScan, copy the whole portable folder.
Remove:  close ArcScan, delete the portable folder. Nothing is left
         behind on the machine: no installer, no service, no ArcScan
         registry settings, no Start-menu entry.


UPDATING
--------

Portable ArcScan does not install updates. It will tell you when a newer
version exists, and updating is four steps:

1. Close ArcScan.
2. Download the newer Windows Portable ZIP for your architecture.
3. Extract it somewhere separate, then copy ArcScan.exe and
   README-PORTABLE.txt over the old ones.
4. Leave ArcScanData exactly where it is.

Launch ArcScan again. It upgrades its own database if the new version
needs to, and your history, names, notes and settings are all still
there.

Do not copy over a running copy of ArcScan.


REQUIREMENTS
------------

* Windows 10 or later, __ARCH__.
* The Microsoft Edge WebView2 Runtime.

WebView2 ships with Windows 11 and with current Windows 10, so it is
almost certainly already present. The portable ZIP deliberately does not
include an installer for it: portable ArcScan does not install system
software. If it is missing, install "Microsoft Edge WebView2 Runtime"
from Microsoft and run ArcScan again.


WHERE PORTABLE ARCSCAN WILL NOT RUN
-----------------------------------

It says so clearly rather than falling back to somewhere you did not
choose:

* A read-only folder, or one it cannot write to. Move the folder to a
  writable local folder or USB drive.

* A network location -- a \\server\share path, or a mapped drive that
  Windows reports as a network drive. Databases over a network share are
  not reliable enough to keep a scan history in. Copy the folder to a
  local or removable drive.

* The same folder as an ArcScan that is already running. Two copies
  sharing one ArcScanData would corrupt it. A second copy from a
  *different* portable folder is fine, and so is running portable
  ArcScan alongside an installed one.

If the drive is removed while ArcScan is running, saving will start
failing and ArcScan will say so. It never writes your data somewhere else
instead.


INSTALLED AND PORTABLE ARCSCAN
------------------------------

They are completely separate and safe to use on the same computer. The
installed edition keeps its data in the normal Windows application-data
location; this copy keeps its data in ArcScanData. Neither reads the
other's history, names, notes or settings, and nothing is merged in
either direction -- not automatically, and not silently.


SIGNING
-------

ArcScan is not code-signed with a paid publisher certificate, so Windows
SmartScreen shows an unknown-publisher warning on first launch. Choose
"More info", then "Run anyway".


PRIVACY
-------

ArcScan scans only what you point it at, stores everything locally, and
sends nothing anywhere. Two optional features contact the internet, both
listed in Settings and both about ArcScan rather than your network: the
update check, and the public-IP lookup, which never runs without a press.

Full notes: https://kingnazz.github.io/ArcScan/privacy.html


MORE
----

Downloads and documentation: https://kingnazz.github.io/ArcScan/
Source and releases:         https://github.com/kingnazz/ArcScan

MIT licensed.
