# ArcScan user guide

Everything ArcScan does, in one document. For a shorter overview see the
[README](../README.md); for the network requests it makes see the
[privacy page](https://kingnazz.github.io/ArcScan/privacy.html).

## Contents

- [Supported operating systems](#supported-operating-systems)
- [Installation](#installation)
- [Your first scan](#your-first-scan)
- [Targets](#targets)
- [Scan profiles](#scan-profiles)
- [Advanced scan settings](#advanced-scan-settings)
- [Live results](#live-results)
- [Reading the results table](#reading-the-results-table)
- [The Inventory](#the-inventory)
- [Present, missing and unknown](#present-missing-and-unknown)
- [Names, status and notes](#names-status-and-notes)
- [Changes](#changes)
- [Change detection](#change-detection)
- [Networks](#networks)
- [History and comparison](#history-and-comparison)
- [Device actions](#device-actions)
- [Exporting](#exporting)
- [The public IP lookup](#the-public-ip-lookup)
- [Settings](#settings)
- [Keyboard shortcuts](#keyboard-shortcuts)
- [Where your data lives](#where-your-data-lives)
- [Upgrading](#upgrading)
- [Uninstalling](#uninstalling)
- [Troubleshooting](#troubleshooting)
- [Known limitations](#known-limitations)
- [Authorisation and responsible scanning](#authorisation-and-responsible-scanning)

## Supported operating systems

| System | Architectures | Installer |
| --- | --- | --- |
| Windows 10 and 11 | x64, ARM64 | NSIS setup (`.exe`) |
| macOS 11 or later | Universal (Apple Silicon and Intel) | Disk image (`.dmg`) |

Linux builds from source and the scanner works there, but no Linux installer is
published and the platform is not part of the release testing.

Scanning needs no administrator or root privileges, because ArcScan uses ordinary
operating system networking rather than raw sockets. The installer itself may ask
for elevation to install for all users, like any application.

## Installation

Download the installer for your platform from the
[latest release](https://github.com/kingnazz/ArcScan/releases/latest) or from the
[website](https://kingnazz.github.io/ArcScan/#download).

ArcScan is not code-signed with a paid publisher certificate and the macOS builds
are not notarised, so both systems warn on first launch:

- **Windows.** SmartScreen shows an unknown-publisher warning. Choose
  **More info**, then **Run anyway**.
- **macOS.** Gatekeeper refuses a double click. Right-click the application and
  choose **Open**, then confirm. This is only needed the first time.

Every release publishes SHA-256 checksums if you want to verify a download before
running it.

## Your first scan

1. Open ArcScan. It detects the subnet this computer is on and fills it into the
   target field.
2. Leave the profile on **Quick LAN**.
3. Press **Scan**, or just press Enter.

Devices appear as they answer. A typical `/24` finishes in a few seconds to about
half a minute, depending on how many devices are on it and how quickly they
reply.

The first scan of a target has nothing to compare against, so it reports no
changes. The second one will.

## Targets

| Form | Example | Notes |
| --- | --- | --- |
| Single address | `192.168.1.20` | |
| CIDR block | `192.168.1.0/24` | Network and broadcast addresses are skipped for `/30` and wider |
| Dashed range | `10.0.0.1-10.0.0.50` | |
| Short dashed range | `10.0.0.1-50` | The end is the last octet |

A target may expand to at most 65,536 addresses. IPv6 is not supported.

## Scan profiles

A profile is a named bundle of ports, timeout and concurrency limits. The profile
is recorded with every scan, and **ArcScan only ever compares scans that share a
target and a profile**, so a Quick LAN sweep is never diffed against a Full TCP
one and no invented port changes appear.

| Profile | For | Behaviour |
| --- | --- | --- |
| **Quick LAN** | A routine look at what is connected | 14 common ports, 700 ms timeout, 64 hosts at once |
| **Reliable LAN** | Phones, printers, Wi-Fi devices, IoT, quiet hosts | 22 ports, 1,600 ms timeout, a gentler 32-host sweep |
| **Full TCP** | A port range you choose | Your ports, with the workload shown before it starts |
| **Remote subnet** | Networks behind a router or VPN | ICMP and TCP only, no local ARP assumptions |
| **Custom** | Everything set by you | Your ports, timeout and all three concurrency limits |

Quick LAN, Reliable LAN and Remote subnet pin their own settings, so their scans
stay comparable with each other over time. Only Custom and Full TCP take the
values from the Advanced panel.

Your last used profile persists, and the default for new sessions is set in
Settings.

## Advanced scan settings

Open **Advanced** in the command bar, or Settings for the persistent defaults.

- **TCP ports.** Lists and ranges, in any mix: `22, 80, 443, 8000-8100`. Up to
  2,048 distinct ports per scan. The backend re-parses and validates whatever the
  field contains, so an invalid or oversized spec is refused rather than
  truncated.
- **Probe timeout.** How long a single probe waits. Longer finds more slow
  devices and takes longer.
- **Host concurrency.** How many addresses are worked on at once.
- **TCP probes.** How many connection attempts exist across the whole scan. This
  is the limit that matters most: pushing it high makes consumer routers drop ARP
  replies, which makes real devices vanish from the results.
- **Ping processes.** How many `ping` child processes run at once. Processes cost
  far more than sockets, so this is the tightest limit.

ArcScan enforces its own ceilings on top of whatever you set, and refuses a scan
whose addresses multiplied by ports exceeds four million connection attempts. It
warns above five hundred thousand and always shows the arithmetic.

## Live results

Devices appear in the table while the scan is still running, and each row fills
in as more is learned about it. A row that is still being resolved shows a marker
rather than a dash, so an incomplete row never reads as an empty one.

The status bar reports devices found, addresses checked, the percentage, the
current phase and elapsed time. The phases are:

1. **Probing addresses.** ICMP and TCP probes across the range.
2. **Confirming quiet devices.** A second ARP pass for local addresses that did
   not answer. This is what makes results consistent between scans, and it is
   skipped entirely for routed targets.
3. **Resolving names and vendors.** Reverse DNS, the ARP table and the
   manufacturer lookup.

Sorting and filtering keep working during a scan, and your sort is preserved as
rows arrive.

**Stop** ends the scan early, in whatever phase it has reached, and keeps
everything found so far — including manufacturer and hostname details that had
already resolved. The results are saved to history like any other scan and marked
as a partial scan; see [Scans you stopped early](#scans-you-stopped-early).
Escape does the same when nothing else is open.

## Reading the results table

| Column | Meaning |
| --- | --- |
| **State** | A dot for online, plus a mark for known, trusted or watched |
| **Name** | Your name for the device, else its hostname, else manufacturer and address |
| **IP address** | |
| **Open services** | Service name and port, for example `HTTPS · 443`. Remote-access services are highlighted |
| **Manufacturer** | From the MAC address, using the full IEEE OUI registry |
| **MAC address** | Local segment only |
| **OS** | Estimated from the reply TTL |
| **Response** | The fastest measurement. Hover for the ICMP and TCP times separately |
| **Last seen** | |

Columns hide themselves as the window narrows, lowest priority first, and can be
turned off individually in Settings. Name, IP address and State are always shown.

Note that **Response** is not a ping time. It is the fastest of the ICMP
round-trip and the TCP connection time, because a device that ignores ICMP still
has a measurable latency. The tooltip shows both.

## The Inventory

The **Inventory** view is every device ArcScan has ever recorded, across every
scan, rather than the devices one scan happened to find. It is where a device's
name, notes, addresses and history live.

Each row carries the device's name, its current address, whether the latest
completed scan found it, its open services, its manufacturer and when it was last
seen. Turn on the MAC, hostname, first-seen, scan-count, response and
previous-address columns in Settings if you want them. Device and Address are
always shown, and lower-priority columns hide themselves before the table can
overflow a narrow window.

**Search** matches the friendly name, hostname, current and previous addresses,
MAC, manufacturer, service names, network name and the opening of your notes. It
is case-insensitive and matches partial words, and every term you type has to
match, so `printer 443` narrows rather than widens.

**Filters** are All, Present, Missing, Unknown, Trusted, Unreviewed and Ignored.
What a scan observed and what you decided about a device are kept apart: a device
you trust can also be missing, and it appears under both.

**Selection** ticks devices for a bulk action: mark trusted, mark unreviewed,
ignore, copy the addresses, or export just those devices. Nothing here deletes
anything. A bulk action over twenty-five devices asks first, and if some of them
have since disappeared it says how many it could not update rather than reporting
a clean success.

Press Enter or double-click a row to open the device panel. The arrow keys, Home
and End move the selection, and Space ticks the focused row.

A device is matched across scans in this order:

1. **MAC address**, normalised so every spelling resolves to the same device.
2. **Hostname plus manufacturer**, for routed targets where ARP gives nothing.
3. **IP address**, as a last resort.

This is why a DHCP lease change reports one device that moved rather than one new
device and one missing device. A device first seen without a MAC is adopted, not
duplicated, once ARP resolves it later, so its name, notes and first-seen date
survive.

## Present, missing and unknown

ArcScan scans when you ask it to. It does not watch a network, so it never says a
device is online or offline. It reports what the most recent completed scan
found, in one of three states.

A network's **reference scan** is its most recent scan that both ran to
completion and recorded which ports it checked. A scan you stopped early is not
one, and neither is a scan recorded by a version of ArcScan old enough not to
have saved its port set.

- **Present** — the reference scan saw the device.
- **Missing** — the reference scan did not see it, but an earlier completed scan
  with the same target and the same ports did. That earlier scan is the evidence
  the reference scan was looking in the right place.
- **Unknown** — nothing can say. The network has no reference scan at all, or the
  device has only ever been seen under different coverage, so its absence from
  the reference scan proves nothing.

A partial scan is excluded from both halves of that rule, which is why stopping a
scan can never turn a device Missing.

Presence is worked out from the stored scans each time the Inventory loads, so it
is always current and never needs rebuilding.

## Names, status and notes

Open a device (double click, or select it and press Enter) to get the device
panel.

- **Name.** Your own name for the device, used everywhere in place of the
  hostname. It follows the device across address changes.
- **Status.** *Not classified*, *Known*, *Trusted*, *Watched* or *Ignored*.
  Status is shown in both tables and can be filtered on. *Ignored* keeps the
  device and everything about it, and takes its changes out of the review inbox;
  they are still recorded and still reachable with the Ignored filter.
- **Notes.** Free text, up to 4,000 characters.

All three live in the database with the device, not in the browser storage v1.6
used, so they survive a reinstall. Labels from v1.6 are imported automatically the
first time v1.7 starts, filling gaps only: a name you have since changed is never
overwritten.

Renaming and reclassifying offer **Undo** for a few seconds.

## Changes

The **Changes** view is a list of everything later scans have turned up, kept
until you have read it. It is not the same thing as the comparison between two
particular scans, which lives in the Scan view and is described under
[Change detection](#change-detection) below.

Each entry says what changed, to which device, on which network and when.
Changes to one device from one scan are shown together, because a device that
moved address and opened a port is one thing that happened, not two.

The filters are Unreviewed (the default), All changes, New devices, Missing
devices, Returned devices, Address changes, Name changes, Service changes,
Acknowledged and Ignored. There is a time filter, a search box and, when you have
more than one network, a network filter.

Each entry offers only the actions that would do something:

- **Review** opens the device panel with the change highlighted, alongside the
  device's addresses, notes and recorded history.
- **Trust** marks the device trusted and acknowledges the new-device entry it was
  offered on, and nothing else about the device.
- **Rename** opens the panel with the name field, and renames the persistent
  device without rewriting anything already recorded.
- **Ignore** marks the device ignored: it stays in the Inventory with its whole
  history, and its changes leave the default inbox.
- **Acknowledge** records that you have read the entry and stamps the time. It
  can be undone.
- **Open the scan** opens the scan that found the change, with its full
  comparison against the baseline. Your filters are still there when you come
  back.

**Acknowledge visible** acknowledges exactly the unreviewed entries currently on
screen, which is why the filters matter: it never reaches anything you cannot
see. Over twenty-five entries it asks first.

Nothing here deletes a record. Acknowledging and ignoring change where an entry
appears, not whether it exists, and everything is included in an export.

If ArcScan was upgraded from an earlier version, the list starts with the scans
you run after the upgrade and says so. Differences found before then are still in
each scan's own comparison, under History.

## Change detection

After each scan, ArcScan compares it with the most recent earlier scan that
covered the same ground and reports:

- New devices, and known devices that returned after being absent
- Devices that have gone missing
- IP address changes
- Hostname, manufacturer and operating-system changes
- Ports that opened and ports that closed

Response times and timestamps are never treated as changes, since they differ on
every scan by nature.

### When two scans are compared

A comparison is only meaningful between scans that looked for the same things in
the same place, so ArcScan requires all four of these to match:

- **The same network.** See [Networks](#networks) below.
- **The same target**, normalised — `192.168.1.0/24` and `192.168.1.37/24` are
  the same subnet.
- **The same ports and discovery mode.** A scan that checked port 22 cannot tell
  you that ports 80 and 443 closed; it never looked at them. Two Custom scans
  with different port lists are therefore not compared, and neither are a local
  ARP-assisted scan and a Remote subnet scan of the same addresses.
- **A completed baseline.** A scan you stopped early is never used as the
  comparison point for another scan.

When ArcScan does not compare two scans it says why rather than implying nothing
changed.

### Scans you stopped early

Stopping a scan saves everything it found, and the scan appears in History marked
**Partial scan** with how many addresses it managed to check. You can open it,
look through it and export it exactly like any other scan.

What a partial scan will never do is report changes. It did not reach every
address, so a device it did not see is not necessarily gone, and a port it did
not probe is not necessarily closed. The comparison explains this instead of
showing differences, the compare action in History is disabled, the next
completed scan compares against the last *completed* scan and skips the partial
one entirely, nothing is added to the Changes list, and no device becomes Missing
in the Inventory.

## Networks

ArcScan keeps a separate inventory for each network you scan. This matters
because private address ranges repeat: a great many offices use
`192.168.1.0/24`, and without separating them, one client's printer at
`192.168.1.20` and another's could be treated as the same device — mixing names,
notes, status and history between unrelated clients.

Networks are identified by the address range together with the default gateway's
hardware address where ArcScan can see it, which is what distinguishes two
different networks that use the same addresses. Device matching, names, notes and
status never cross between them.

Open **Settings → Networks** to give each one a name — `Head office`,
`Client VPN`, `Warehouse` — and History will show which network each scan
belongs to.

The **Changes** tab shows the full comparison with added, missing and changed
devices kept separate and field-level differences for each change. Every entry
carries an icon and a word as well as a colour.

Targets are normalised before they are compared, so `192.168.1.0/24` and
`192.168.1.37/24` are the same network, and `10.0.0.1-50` and
`10.0.0.1-10.0.0.50` are the same range. A single address, a range and a CIDR
block stay distinct even when they cover the same addresses.

## History and comparison

The **History** tab lists every saved scan with its target, profile, network,
date, duration, address count, device count and change counts. From each entry
you can open the scan, compare it with the preceding compatible one, export it,
or delete it. Deleting asks first.

Comparing is disabled when there is nothing safe to compare against — for a
partial scan, or when no earlier completed scan checked the same target with the
same ports — and the button says which it is.

History retention defaults to the newest 100 scans and is set in Settings. Pruning
removes scans only: device names, notes and first-seen dates are always kept.

## Device actions

Available from the device panel. Only the actions the open services support are
enabled, and a disabled one says why.

| Action | Needs | Notes |
| --- | --- | --- |
| **Copy IP** | Nothing | Always available |
| **Open web interface** | 443, 8443, 80, 8080, 8000 or 8081 | HTTPS is preferred |
| **Open shared folders** | 445 or 139 | Explorer on Windows, `smb://` elsewhere |
| **Open Remote Desktop** | 3389 | `mstsc` on Windows, the `rdp://` handler on macOS |
| **Open SSH** | 22 | Opens a visible terminal |
| **Wake-on-LAN** | A MAC address | Broadcast magic packet to UDP 255.255.255.255:9 |

Addresses are validated as bare IPv4 before they reach any launcher, and no value
is ever passed through a shell.

Wake-on-LAN only wakes a device that has Wake-on-LAN enabled in its own firmware
or operating system settings, and it does not cross a router.

## Exporting

Every export writes CSV, JSON or XML through the native save dialog, and each one
says what it is about to contain before you pick a format.

**From the Scan view**, or Ctrl/Cmd + E while it is open, the export contains the
devices currently shown, so a filter narrows it. Columns: name, IP, hostname,
MAC, vendor, OS, TTL, open ports, response, ICMP, TCP, status and last seen. The
v1.6 columns are all still there in the same order, with the two latency
measurements appended, so existing spreadsheets and scripts keep working.

**From the Inventory** the scope follows what you are looking at: the selected
devices if you have ticked any, otherwise the filtered set, which may be one
network or all of them. Columns: network, device, status, presence, current IP,
previous IPs, MAC, manufacturer, hostname, OS guess, open ports, open services,
first seen, last seen, observations and notes. Presence and status are written
out as words rather than as internal values. Filenames carry the day and, where
the export is scoped to one network, its name:
`arcscan-inventory-home-wi-fi-2026-08-03.csv`.

**From Changes** the export contains exactly the entries on screen, ignored ones
included when the Ignored filter is showing them. Columns: date, network, device,
IP, MAC, change, previous value, new value, opened ports, closed ports, scan,
baseline, review state and acknowledged date.

Exporting from **History** is different again, and deliberately so: it writes
exactly the scan whose row you used, in the format you pick from that row,
regardless of which scan is currently displayed or how the table is filtered. The
filename carries that scan's own target. Your current view is left as it was.

Internal identifiers stay out of CSV and XML. The JSON form keeps the device or
event id, which is the only place a script has any use for one.

## The public IP lookup

ArcScan can report the address your internet connection appears from. It is
**off by default** and never runs on its own.

Enable it in **Settings, Network requests**, then press **Check public IP**. The
lookup contacts `api64.ipify.org` and, if that fails, `icanhazip.com`. Nothing but
the request itself is sent: no target, no result, no device data. The answer is
held for the current session only and is forgotten when you close ArcScan or press
Forget.

v1.6 performed this lookup automatically at startup. v1.7 does not.

## Settings

| Group | Contains |
| --- | --- |
| Appearance | Theme (system, light, dark), row density, reduced motion |
| Scanning | Default profile, ports, timeout, all three concurrency limits |
| Results | Which columns are visible |
| History | Scans to keep, change notifications |
| Network requests | Public IP lookup, update checks |
| Getting started | Whether the first-run guidance is shown |

## Keyboard shortcuts

| Key | Action |
| --- | --- |
| Enter | Start the scan, from the target field |
| Escape | Close settings, then the device panel, then the current view's selection, then its filters, then stop the scan |
| Ctrl/Cmd + F, or `/` | Focus the search box of the view you are in |
| Ctrl/Cmd + L | Focus the target field |
| Ctrl/Cmd + R | Rescan the last target |
| Ctrl/Cmd + E | Export what the current view is showing |
| Arrow keys, Home, End | Move the selection in a table |
| Space | Tick or untick the focused row in the Inventory |
| Enter | Open the selected device |

Escape's order is deliberate and fixed: it never cancels a scan while something is
open on top of the results, and it clears a selection before it clears filters,
because the selection is the more recent and more surprising thing to leave
behind.

## Where your data lives

| System | Path |
| --- | --- |
| Windows | `%APPDATA%\com.arcscan.app\arcscan.db` |
| macOS | `~/Library/Application Support/com.arcscan.app/arcscan.db` |

It is an ordinary SQLite file. You can copy it, back it up, inspect it with any
SQLite tool, or delete it to start over. Interface preferences are stored
separately by the application window and are not in this file.

## Upgrading

Install v1.8.0 over v1.7.x or v1.6.x without deleting anything. On first launch
ArcScan migrates the database in place.

From v1.7.1 the upgrade is small: the Inventory and the presence states are
computed from the scans you already have, so they are populated the moment you
open the view, and nothing is rebuilt or rewritten. The only new storage is the
record behind the Changes list, and it starts empty on purpose. Replaying every
past comparison would be unbounded work at launch and would greet you with a
backlog of changes you were never asked to review; those differences are still in
each scan's own comparison, under History. The list fills from your next
completed scan onwards, and says so until it does.

From v1.6.x or v1.7.0 the earlier migrations run first:

- Every scan and every observation it already held is kept.
- The device inventory is built from those existing observations, oldest scan
  first, so first-seen dates are truthful from day one.
- Scan targets are normalised so older scans become comparable.
- Device labels from v1.6's browser storage are imported.
- Existing devices and scans are placed into networks, keeping their ids, names,
  notes, status and dates. Networks are refined as you scan: ArcScan learns each
  one's gateway on the next scan of it, and you can name them in Settings.
- Every scan is given a coverage signature. For the fixed profiles this is known
  exactly. For Custom and Full TCP scans recorded by earlier versions it is not —
  those versions never saved which ports were checked — so those scans are kept
  and readable but are not compared. A missing comparison is better than a wrong
  one.

Migrations are idempotent and transactional: opening the same database repeatedly
changes nothing, and an interrupted upgrade leaves it exactly as it was.

ArcScan checks for updates on launch and offers a one-click install. The update
package is cryptographically signed and the signature is verified before
installing. The check can be switched off in Settings.

## Uninstalling

Uninstall through Windows Settings or by deleting the application on macOS. The
database is deliberately left behind, so a reinstall keeps your history, names and
notes. To remove it, delete the file at the path above.

## Troubleshooting

**A device I know is there does not appear.**
Try the Reliable LAN profile. It waits longer, probes a wider port set and
re-triggers ARP resolution for addresses that did not answer. Some devices ignore
ICMP entirely and have no open ports, and an access point with client isolation
prevents one device from reaching another at all.

**Results differ between scans of the same network.**
That is what the confirmation pass exists to reduce. If it persists, lower the
TCP probes limit: consumer routers rate-limit and drop ARP replies under heavy
fan-out, which makes real devices disappear. A gentler sweep finds more of them.

**No MAC addresses or manufacturers.**
MAC addresses are only visible on your own network segment. A target on the other
side of a router has none to read, so devices there are matched by hostname and
manufacturer or by address.

**Everything in the range looks like it is up.**
Some routers and access points answer ARP for every address in a subnet with
their own MAC. ArcScan detects that and discards it, so those addresses are not
reported as devices. If you still see it, the intercepting device is answering
TCP as well, which no scanner can distinguish from a real host.

**The scan is slow.**
A large port range across many addresses is a lot of connection attempts. The
command bar shows the total before you start. Narrow the range, narrow the ports,
or use Quick LAN.

**Windows or macOS warns when I run the installer.**
Expected: ArcScan is not publisher-signed. See [Installation](#installation).

**Wake-on-LAN does nothing.**
The device must have Wake-on-LAN enabled in its own settings, and the magic packet
is a broadcast, so it does not cross a router.

## Known limitations

- **IPv4 only.** IPv6 discovery is not implemented.
- **No continuous monitoring.** ArcScan scans when you ask and compares with the
  previous scan. It does not watch a network or send alerts. Present and missing
  describe what the latest completed scan found, not what is true right now.
- **Presence needs a completed scan.** Until a network has one that also recorded
  which ports it checked, every device on it reads Unknown.
- **Search reaches the opening of a note**, not the whole of it. The Inventory
  loads a short excerpt so search can find it without pulling every note body
  into the table.
- **The Changes list starts at the upgrade** on an existing installation, rather
  than being backfilled from older scans.
- **Not a vulnerability scanner.** It reports what is reachable and makes no
  assessment of whether anything is vulnerable.
- **TCP only for ports.** There is no UDP port scanning.
- **The OS guess is a guess.** It comes from the reply TTL and is wrong for
  anything that changes its default TTL.
- **No installers for Linux**, although it builds and runs there.
- **MAC addresses need the local segment**, so routed scans identify devices less
  precisely.
- **No code signing yet**, so both systems warn on first launch.

## Authorisation and responsible scanning

ArcScan performs read-only discovery. It sends ICMP echo requests and TCP
connection attempts, and reads your own computer's ARP table. It never attempts a
password, never sends an exploit, and has no stealth or evasion behaviour. None of
that will be added.

Scan only networks you own or have been explicitly authorised to inspect.
Scanning a network without permission may be unlawful where you are, whatever
tool is used and however read-only it is. On a network you are authorised to
scan, tell whoever runs its monitoring first: a sweep across a subnet looks
exactly like reconnaissance to an intrusion detection system, because at the
packet level it is the same thing.
