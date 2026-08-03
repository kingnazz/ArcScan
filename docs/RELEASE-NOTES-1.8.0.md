# ArcScan 1.8.0

**Scan a network, understand what is connected, and keep track of what changes.**

v1.7.1 made ArcScan trustworthy about a single scan. v1.8.0 makes it useful
across many: everything it has ever found is now in one place, with an honest
answer about which of it the last completed scan reached, and a list of changes
that stays until you have actually read it.

Install over v1.7.x or v1.6.x without losing anything. The database migrates in
place and keeps every scan, device, name, note, status, network and date it
already held.

---

## Navigation

`Scan · Inventory · Changes · History`

**Scan** is unchanged: live results, the completed scan's table, its comparison
with the previous scan, and the device panel. **History** is unchanged. The two
new views sit alongside them, and none of the four is ever disabled — each
explains its own empty state, which is more useful than a tab you cannot click.

## Inventory

Every device ArcScan has recorded, across every scan, rather than the devices one
scan happened to find.

- **One row per device**, with its name, current address, presence, open
  services, manufacturer, network and when it was last seen. MAC, hostname,
  first seen, scan count, response time and previous address are available as
  optional columns. Device and Address are always shown, and lower-priority
  columns hide themselves before the table can overflow a narrow window.
- **Search** matches the friendly name, hostname, current and previous addresses,
  MAC, manufacturer, service names, network name and the opening of your notes.
  Case-insensitive, partial, and every term has to match, so `printer 443`
  narrows rather than widens.
- **Filters**: All, Present, Missing, Unknown, Trusted, Unreviewed, Ignored. What
  a scan observed and what you decided about a device are kept apart, so a device
  you trust can also be missing and appears under both.
- **Selection and bulk actions**: mark trusted, mark unreviewed, ignore, copy the
  addresses, export the selection. Nothing here deletes anything. Large actions
  confirm first, and a partial failure is reported rather than dressed up as a
  success.
- **Keyboard**: arrow keys, Home and End move the selection, Space ticks the
  focused row, Enter opens the device panel — the same model the scan results
  table has always used.

## Present, missing and unknown

ArcScan scans when you ask it to. It does not watch a network, so it never says a
device is online or offline.

A network's **reference scan** is its most recent scan that both ran to
completion and recorded which ports it checked.

- **Present** — the reference scan saw the device.
- **Missing** — the reference scan did not, but an earlier completed scan with
  the same target and the same ports did. That earlier scan is the evidence the
  reference scan was looking in the right place.
- **Unknown** — nothing can say: no reference scan for the network, or the device
  has only ever been seen under different coverage.

A scan you stopped early is excluded from both halves of that rule, which is why
stopping a scan can never turn a device Missing.

## Changes

A list of everything later scans have turned up, kept until you have read it.
This is not the same thing as the comparison between two particular scans, which
is still in the Scan view.

- Each entry says what changed, to which device, on which network and when.
  Changes to one device from one scan are shown together, because a device that
  moved address and opened a port is one thing that happened.
- Filters: Unreviewed (the default), All changes, New devices, Missing devices,
  Returned devices, Address changes, Name changes, Service changes, Acknowledged
  and Ignored. Plus a time filter, search, and a network filter when you have
  more than one network.
- Actions are offered only where they would do something: **Review**, **Trust**,
  **Rename**, **Ignore**, **Acknowledge** (undoable) and **Open the scan**, which
  reaches the full comparison and leaves your filters intact when you come back.
- **Acknowledge visible** affects exactly the unreviewed entries on screen, never
  anything the filters are hiding, and confirms above twenty-five.
- Nothing here deletes a record. Acknowledging and ignoring change where an entry
  appears, not whether it exists.

## Networks, and optional names

Network separation stays what it was in v1.7.1: an automatic correctness
mechanism, resolved from the canonical network and the gateway's MAC address, so
two unrelated `192.168.1.0/24` networks never mix their devices, names or notes.

v1.8.0 puts it to work where it helps. Networks can be given names — `Home
Wi-Fi`, `Office`, `Workshop` — and those names appear in the Inventory, in
Changes, in History and in the device panel. An unnamed network shows its address
range instead. With a single network, the Network column and the network filters
are hidden entirely.

Renaming a network reaches every view at once.

## Exports

- **Inventory**: the selection if you have one, otherwise the filtered set, which
  may be one network or all of them. Network, device, status, presence, current
  IP, previous IPs, MAC, manufacturer, hostname, OS guess, open ports, open
  services, first seen, last seen, observations and notes. Presence and status
  are written out as words, not internal values.
- **Changes**: exactly the entries on screen. Date, network, device, IP, MAC,
  change, previous value, new value, opened ports, closed ports, scan, baseline,
  review state and acknowledged date.
- Both come in CSV, JSON and XML, with dated filenames that carry the scope:
  `arcscan-inventory-home-wi-fi-2026-08-03.csv`. Each menu says what the export
  will contain before you pick a format.
- Internal identifiers stay out of CSV and XML. The JSON form keeps the device or
  event id, which is the only place a script has any use for one.

The scan and History exports are unchanged.

## Upgrading from 1.7.1

The Inventory and the presence states are computed from the scans you already
have, so they are populated the moment you open the view. Nothing is rebuilt or
rewritten.

The only new storage is the record behind the Changes list, and **it starts
empty on purpose**. Replaying every past comparison would be unbounded work at
launch and would greet you with a backlog of changes you were never asked to
review. Those differences are still in each scan's own comparison, under History.
The list fills from your next completed scan onwards, and says so until it does.

The migration is transactional and idempotent: opening the same database
repeatedly changes nothing, and an interruption leaves it exactly as it was.

## Under the hood

- The Inventory is a **query**, not a second copy of the data. A rename, a status
  change or a deleted scan can never leave two versions of the truth.
- It costs **two statements** whatever the size of the database. The latest
  observation, the observation count, the presence verdict and the previous
  addresses are computed set-wise rather than by asking a question per device,
  and notes are reduced to a short excerpt because the table shows an indicator
  and a search term, not a body.
- Change events are **deterministic**: the key is the scan, the device and the
  kind of change, with a unique index behind it, so a retried save or a reopened
  scan cannot produce a duplicate.
- Port changes keep the opened and closed lists **as numbers**, not only as
  display text.
- Change records are **not** foreign-keyed to scans, and carry the scan dates and
  a device label of their own, so a change stays readable after retention prunes
  the scan that found it.
- Marking a device **Ignored** keeps the device and its whole history, and records
  its future changes already-ignored so they stay out of the default inbox
  without being lost.

## Accessibility and layout

Inventory and Changes are keyboard accessible and meet WCAG AA in both themes.
The browser suite runs axe-core across every view, the device panel, an active
selection and the no-matches state, in dark and light, and checks four widths
from 1440 down to 940 for horizontal overflow. Two real problems it found are
fixed: the clear buttons on the new search fields are a full 24px target, and
muted text inside a selected row steps up to the secondary tone, which it needed
in the light theme whether or not this release existed.

## Not in this release

No scheduled scans, background scanning, notifications, tray mode or launch at
login. No mDNS, SSDP or SNMP. No credential storage, IPv6, UDP port scanning,
code signing or macOS notarisation. No cloud accounts, sync, remote agents, web
dashboards, team features or ticketing.

ArcScan remains a general-purpose network scanner for one person and their
networks.

## Known limitations

- Presence needs a completed scan that also recorded its port set. Until a
  network has one, every device on it reads Unknown.
- Search reaches the opening of a note (160 characters), not the whole of it.
- The Changes list starts at the upgrade on an existing installation.
- The inbox loads the most recent 5,000 changes; older ones are kept and appear
  in an export.
- Everything true of v1.7.1 is still true: IPv4 only, TCP-based port scanning,
  no continuous monitoring, MAC addresses only on the local segment, and
  unsigned installers on both platforms.
