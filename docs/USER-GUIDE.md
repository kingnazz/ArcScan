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
- [The device inventory](#the-device-inventory)
- [Names, status and notes](#names-status-and-notes)
- [Change detection](#change-detection)
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

**Stop** ends the scan early and keeps everything found so far, which is saved to
history like any other scan and marked as stopped. Escape does the same when
nothing else is open.

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

## The device inventory

Every scan folds its results into a persistent inventory. A device is matched
across scans in this order:

1. **MAC address**, normalised so every spelling resolves to the same device.
2. **Hostname plus manufacturer**, for routed targets where ARP gives nothing.
3. **IP address**, as a last resort.

This is why a DHCP lease change reports one device that moved rather than one new
device and one missing device. A device first seen without a MAC is adopted, not
duplicated, once ARP resolves it later, so its name, notes and first-seen date
survive.

## Names, status and notes

Open a device (double click, or select it and press Enter) to get the device
panel.

- **Name.** Your own name for the device, used everywhere in place of the
  hostname. It follows the device across address changes.
- **Status.** *Not classified*, *Known*, *Trusted* or *Watched*. Status is shown
  in the results table and can be filtered on.
- **Notes.** Free text, up to 4,000 characters.

All three live in the database with the device, not in the browser storage v1.6
used, so they survive a reinstall. Labels from v1.6 are imported automatically the
first time v1.7 starts, filling gaps only: a name you have since changed is never
overwritten.

Renaming and reclassifying offer **Undo** for a few seconds.

## Change detection

After each scan, ArcScan compares it with the most recent earlier scan of the
same target and profile and reports:

- New devices, and known devices that returned after being absent
- Devices that have gone missing
- IP address changes
- Hostname, manufacturer and operating-system changes
- Ports that opened and ports that closed

Response times and timestamps are never treated as changes, since they differ on
every scan by nature.

The **Changes** tab shows the full comparison with added, missing and changed
devices kept separate and field-level differences for each change. Every entry
carries an icon and a word as well as a colour.

Targets are normalised before they are compared, so `192.168.1.0/24` and
`192.168.1.37/24` are the same network, and `10.0.0.1-50` and
`10.0.0.1-10.0.0.50` are the same range. A single address, a range and a CIDR
block stay distinct even when they cover the same addresses.

## History and comparison

The **History** tab lists every saved scan with its target, profile, date,
duration, address count, device count and change counts. From each entry you can
open the scan, compare it with the preceding compatible one, export it, or delete
it. Deleting asks first.

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

**Export** in the results toolbar, or Ctrl/Cmd + E, writes CSV, JSON or XML
through the native save dialog. The export contains the devices currently shown,
so a filter narrows it.

Columns: name, IP, hostname, MAC, vendor, OS, TTL, open ports, response,
ICMP, TCP, status and last seen. The v1.6 columns are all still there in the same
order, with the two latency measurements appended, so existing spreadsheets and
scripts keep working.

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
| Escape | Close settings, then the device panel, then clear the filter, then stop the scan |
| Ctrl/Cmd + F, or `/` | Focus the filter |
| Ctrl/Cmd + L | Focus the target field |
| Ctrl/Cmd + R | Rescan the last target |
| Ctrl/Cmd + E | Export |
| Arrow keys, Home, End | Move the selection in the table |
| Enter | Open the selected device |

Escape's order is deliberate and fixed: it never cancels a scan while something is
open on top of the results.

## Where your data lives

| System | Path |
| --- | --- |
| Windows | `%APPDATA%\com.arcscan.app\arcscan.db` |
| macOS | `~/Library/Application Support/com.arcscan.app/arcscan.db` |

It is an ordinary SQLite file. You can copy it, back it up, inspect it with any
SQLite tool, or delete it to start over. Interface preferences are stored
separately by the application window and are not in this file.

## Upgrading

Install v1.7 over v1.6.4 without deleting anything. On first launch ArcScan
migrates the database in place:

- Every scan and every observation it already held is kept.
- The device inventory is built from those existing observations, oldest scan
  first, so first-seen dates are truthful from day one.
- Scan targets are normalised so older scans become comparable.
- Device labels from v1.6's browser storage are imported.

Migrations are idempotent: opening the same database repeatedly changes nothing.

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
  previous scan. It does not watch a network or send alerts.
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
