# Changelog

All notable changes to ArcScan. This project follows
[semantic versioning](https://semver.org/spec/v2.0.0.html).

## [1.8.2] - 2026-08-18

Better device discovery. ArcScan now asks the local network what its devices
are, using the two protocols printers, televisions, routers, cameras and media
equipment already speak, and shows what it hears along with where each answer
came from. Install over 1.8.x, 1.7.x or 1.6.x without losing anything. Full
notes: [docs/RELEASE-NOTES-1.8.2.md](docs/RELEASE-NOTES-1.8.2.md).

### Added

- **mDNS discovery.** ArcScan asks the local link which services exist on it and
  follows up only on what the link named, rather than sending a fixed list of
  guesses. It speaks as a one-shot querier — an ephemeral port, a few seconds of
  listening, then the socket closes — so it never binds port 5353, never
  collides with the Bonjour or Avahi responder already running, never answers a
  query, and keeps nothing between scans.
- **SSDP discovery.** A standards-compliant `M-SEARCH` with a small `MX`. Where
  a device advertises a description document, ArcScan reads it for the
  manufacturer and model, subject to the URL rules below.
- **Better detected names.** A device that publishes `Acme LaserFast 400` is no
  longer shown as its address. The order is fixed: a name you typed, then a
  high-confidence mDNS name, then a high-confidence SSDP friendly name, then the
  reverse-DNS hostname, then the manufacturer with an established type, then an
  mDNS host name, then the address. **A name you typed always wins**, before any
  other rule is consulted. Names that describe a category rather than a device
  (`printer`, `android`, `UPnP Device`) are demoted below the hostname.
- **Device types with confidence and evidence.** Fourteen types and four words —
  high, medium, low, unknown — rather than a score, because there is no sense in
  which a printer service is 0.7 of a printer. The drawer shows what the verdict
  rests on, and where the evidence supports two answers it shows both.
- **Discovery sources.** Every detected name, model and type records which
  protocol it came from, and the Inventory, the drawer and the export all say so.
- **Five optional Inventory columns** (Type, Detected name, Model, Discovered by,
  Last discovered) and a device-type filter, all off by default. Search reaches
  detected names, models, types and services, and indexes each service under both
  its protocol name and its friendly one, so `_ipp._tcp` and "IPP printing" both
  find the printer.
- **A Local discovery section in Settings**: a master switch, and under it
  whether description documents may be read. Both on by default.
- **Eight new Inventory export columns.** A device no discovery-capable scan has
  reached exports blank cells rather than the word "Unknown", which would read as
  an answer.
- **A deterministic discovery demo.** `?discovery=normal|none|conflict|malformed|slow`
  drives the browser demo through the real merge, naming and rendering code. The
  malformed case has devices advertise script tags, control characters and an
  80-character service name, so the interface's handling of hostile input is
  something the suite checks rather than something the notes claim.

### Security

- **SSDP `LOCATION` is treated as hostile input.** Refused outright: any scheme
  but plain `http`, embedded credentials, fragments, IPv6 literals, ports 0, 22,
  23, 25, 465 and 587, control characters anywhere, and anything past a length
  cap. The destination must then be inside the local network the scan actually
  ran against — loopback, link-local (including the cloud-metadata address),
  multicast, broadcast and *other private subnets* are all refused. A host name
  answering with one local and one public address is refused entirely. Unusual
  address spellings (`0x7f.0.0.1`, `2130706433`, `0177.0.0.1`) are normalised
  before the check.
- **No DNS-rebinding window.** The approved address is what the connection uses;
  there is no second resolution between the check and the connect.
- **No redirects and no proxy.** A `3xx` is an error, not an instruction, and the
  fetcher builds its own connection rather than consulting a system proxy that
  would send the request to a host nothing validated.
- **`https` descriptions are refused deliberately.** A local device serves one
  under a self-signed certificate for an address, which nothing can verify;
  supporting it would mean a TLS stack with verification switched off. The SSDP
  headers still carry a manufacturer and a device type.
- **Description documents allow no DTD.** A document containing `<!DOCTYPE` is
  refused before a field is read, which removes external entities, parameter
  entities and expansion bombs in one rule. Only the five predefined entities and
  numeric character references decode. Nesting is capped with an explicit stack
  rather than recursion, and document size, field length, service count and
  element count are all bounded. Every value is rendered as plain text.
- **No new webview permissions.** Multicast and description fetching are entirely
  in Rust, so the Content Security Policy and the Tauri capability set are
  unchanged.

### Changed

- **Discovery is local-only, and uncertainty means not sending.** It runs only
  when the target is inside a subnet this computer is attached to. Remote-subnet
  scans, routed targets and public targets never send a multicast packet, and the
  multicast TTL is 1 so nothing leaves the local link regardless of how a router
  is configured.
- **The Changes inbox stays quiet.** Discovery records events only for a
  materially changed high-confidence name, a changed high-confidence type, a
  meaningful service appearing or disappearing, and a manufacturer or model
  change. Nothing is recorded for the first sighting of a device, for anything
  below high confidence, for whitespace or casing, for TTLs, `CACHE-CONTROL`,
  `BOOTID`, `CONFIGID`, `SEARCHPORT` or `SERVER` banners, for TXT ordering, or
  for a failed description fetch. A service is only reported as gone after **two
  consecutive** discovery-complete scans miss it, because one missed multicast
  response is ordinary on Wi-Fi.
- **A stopped scan records no discovery events**, and a scan that ran discovery
  is never compared on discovery-derived facts against one that could not.
- **New scan phases** — Discovering local services, Reading device descriptions,
  Classifying devices — shown in the progress strip. Stop interrupts every one of
  them, keeping whatever had already been parsed.
- **Device identity is untouched.** Matching is still MAC, then
  hostname-and-vendor, then address, scoped to a network. A detected name is
  evidence attached to a device, never a key; it never merges devices and never
  crosses a network scope, and neither does an mDNS instance name or a UPnP UDN.
  Presence semantics and port comparison compatibility are unchanged.
- **An IPv6 address learned from mDNS is shown as supplemental information**,
  with a note saying so. ArcScan scans IPv4 only, and showing an address is not a
  claim that anything was scanned at it.

### Fixed

- **A newly published advisory against `nanoid`** (GHSA-2v37-7h3g-55p8), which
  arrives transitively under postcss and runs at build time only. A lockfile bump
  within the existing range.

## [1.8.1] - 2026-08-04

Visual polish and the public-IP lookup on the Scan screen. No scanner,
Inventory, Changes or database change: this release is about the window and
where one existing feature lives. Full notes:
[docs/RELEASE-NOTES-1.8.1.md](docs/RELEASE-NOTES-1.8.1.md).

### Fixed

- **The duplicate ArcScan title and icon.** The packaged app showed the product
  name and mark twice, stacked: once in the title bar the operating system
  draws, and again in the app's own toolbar below it. The in-app block is
  removed and the native title bar kept, so the OS keeps providing the window
  controls, snapping, full-screen behaviour, keyboard window management and
  accessibility hooks. The view switcher takes the start of the row, with no
  blank strip where the brand block used to be. A browser has no native title
  bar, which is why this was invisible in development and in every automated
  check; the browser suite now asserts the app renders no title or icon of its
  own.
- **Two contrast failures that predate this release**, both now clearing WCAG AA
  on every surface they appear on: the count badges beside the view tabs, and
  the Stop button, which sat at 4.3:1 against its own hover tint for as long as
  a scan was running.
- **Two controls both called "Clear the filter"** in the results toolbar, now
  "Clear the search" and "Clear all filters". The second is disabled rather than
  hidden when there is nothing to clear.

### Added

- **Public IP on the Scan screen.** A compact utility in a new context row under
  the view switcher, beside the local network the scan will run against. It
  shows the address, how long ago it was checked, and offers Copy and Refresh;
  a failure says so plainly, offers Retry, and keeps the raw provider errors
  behind Technical details.
- **A deterministic public-IP demo.** `?publicip=ok|fallback|fail|flaky|slow`
  drives the browser demo's scripted providers through the real fallback code,
  so first-provider failure, total failure, retry and a slow lookup are all
  reachable without a network. The demo makes no outbound request at all, and
  the addresses it shows are RFC 5737 documentation addresses.

### Changed

- **The public-IP lookup still never runs on its own.** Nothing is looked up at
  startup, when the Scan screen opens, after a scan, on a view change, on a
  timer or in the background; only Check, Refresh and Retry contact a provider.
  The answer stays in memory for the session, is written to neither the database
  nor your preferences, and is in no export.
- **Settings keeps the explanation, not a second set of controls.** Checking,
  copying and refreshing moved to the Scan screen; Settings retains the privacy
  copy, the session value and Forget.
- **The "Allow public IP lookups" switch is now "Offer the public IP lookup"**
  and is on for new installs. It has only ever decided whether the control is
  offered, never whether a request happens by itself. A preference explicitly
  turned off in an earlier version is respected and left off.
- **Repeated presses no longer stack up lookups**, a response that arrives after
  the value was forgotten or after a newer lookup started is discarded, and a
  provider that never answers now fails with a timeout instead of leaving the
  control spinning.

## [1.8.0] - 2026-08-03

A persistent Inventory, honest presence states, and a Changes list that stays
until you have read it. Navigation becomes `Scan · Inventory · Changes · History`.

Install over 1.7.x or 1.6.x without losing anything. The database migrates in
place and keeps every scan, device, name, note, status, network and date it
already held. Full notes: [docs/RELEASE-NOTES-1.8.0.md](docs/RELEASE-NOTES-1.8.0.md).

### Added

- **A persistent Inventory.** Every device ArcScan has recorded, across every
  scan, in one searchable list: name, current address, presence, services,
  manufacturer, network and last seen by default, with MAC, hostname, first seen,
  scan count, response time and previous address available as optional columns.
  Search matches names, hostnames, current and previous addresses, MACs,
  manufacturers, service names, network names and the opening of your notes.
  Filters cover All, Present, Missing, Unknown, Trusted, Unreviewed and Ignored.
  Selection supports marking trusted, marking unreviewed, ignoring, copying
  addresses and exporting the selection; nothing there deletes anything.
- **Present, missing and unknown.** Presence is decided only from a network's
  most recent scan that both completed and recorded which ports it checked.
  Present means that scan saw the device; missing means it did not, and an
  earlier completed scan with the same target and coverage did; unknown means
  nothing can say. ArcScan never claims a device is online or offline, and a
  partial scan can never make a device missing.
- **A Changes list.** New devices, returning devices, missing devices, address,
  hostname, manufacturer, operating-system and MAC changes, and services that
  opened or closed, kept as records rather than appearing in one comparison and
  vanishing. Changes to one device from one scan are grouped. Review, Trust,
  Rename, Ignore and Acknowledge are offered only where they would do something,
  Acknowledge is undoable, and **Acknowledge visible** affects exactly the entries
  on screen. Ignored entries leave every view except the Ignored filter; nothing
  is deleted.
- **Optional network names.** Networks can be named — `Home Wi-Fi`, `Office`,
  `Workshop` — and the name appears in the Inventory, Changes, History and the
  device panel. An unnamed network shows its address range. Renaming reaches
  every view at once.
- **Inventory and Changes exports** in CSV, JSON and XML, scoped to the
  selection, the current filter, one network or everything, with dated filenames
  that carry the scope. Presence and status are written out as words. Internal
  identifiers stay out of CSV and XML.
- **An `Ignored` device status**, which keeps the device and its history and
  takes its changes out of the review inbox.
- **Bulk device classification** and a note-fetch command used only by exports.

### Changed

- Navigation is now `Scan · Inventory · Changes · History`, and no view is ever
  disabled. The scan-to-scan comparison stays inside Scan, where it belongs: it
  describes one scan against one baseline, while Changes is the persistent list
  across every scan.
- The scan toolbar always offers the comparison, reading **Why no comparison?**
  when there is none — so a scan stopped early, the case where the explanation
  matters most, can finally reach it.
- Escape now resolves against the view in front of you: settings, then the device
  panel, then that view's selection, then its filters, and only then a running
  scan. The search and export shortcuts follow the view you are in instead of
  jumping back to the scan results.
- The device panel shows presence, network, last seen and the recorded changes
  for the device, and distinguishes a scan's observation from the persistent
  record. Opened in a scan it says the device answered that scan; opened from the
  Inventory it reports presence.
- Renames, classifications and network renames refresh the Inventory and the
  Changes list in place. Nothing reloads the application, so a scan in progress,
  the filters and the selection all survive.
- The browser demo now carries two networks with their own gateways and ten kinds
  of device, produces all three presence states honestly, and includes devices
  nothing could identify. `?demo=empty` starts it with no history.

### Fixed

- Muted text inside a selected table row was below the 4.5:1 contrast floor in
  the light theme. It now steps up to the secondary tone.
- The clear buttons on the new search fields are a full 24px target.

### Database

- Schema version 4. One new table, `change_events`, with a unique key per scan,
  device and kind of change, so a retried save or a reopened scan cannot create a
  duplicate. Port changes store the opened and closed lists as numbers, not only
  as display text.
- Change records are deliberately not foreign-keyed to scans and carry the scan
  dates and a device label of their own, so they stay readable after retention
  prunes the scan that found them.
- The Inventory is a query rather than a materialised table, so a rename, a
  status change or a deleted scan can never leave two copies of the truth. It
  costs two statements whatever the size of the database.
- The upgrade records a watermark and starts the Changes list empty rather than
  replaying every historical comparison, which would be unbounded work at launch
  and would create a backlog nobody asked to review. Those differences are still
  in each scan's own comparison.

## [1.7.1] - 2026-08-01

A correctness, reliability and security hardening release. No new product
surface: v1.7.1 fixes the ways v1.7.0 could tell you something untrue about a
network, and tightens what the application is allowed to do.

Install over 1.6.x or 1.7.0 without losing anything. The database migrates in
place, in one transaction, and keeps every scan, device, name, note, status and
date it already held.

### Fixed

- **Stop now stops the scan, in every phase.** Cancellation was captured once,
  after the address sweep. A scan stopped during ARP settling, quiet-device
  confirmation or hostname resolution carried on doing that expensive work and
  was then recorded as *completed*. Cancellation is re-checked before every
  phase, during both settle waits — which now end the moment Stop is pressed,
  rather than running out a fixed delay — and before each reverse-DNS lookup is
  launched. The final `cancelled` state is decided at the end of the scan, so a
  stopped scan is never saved as a complete one. Hosts already found, and any
  MAC, manufacturer and hostname already resolved, are kept.
- **A stopped scan no longer invents missing devices or closed ports.** A
  cancelled scan has not seen its whole target, so absence from it proves
  nothing. Partial scans are still saved, opened and exported in full, but they
  are never compared and never become the baseline for another scan. A completed
  scan skips any newer cancelled scan and compares against the last completed
  one instead. History labels them **Partial scan** and says why changes are
  unavailable.
- **Scans that checked different ports are no longer compared.** Compatibility
  depended on the target and the profile *name*, so two `Custom` scans of the
  same subnet — one probing 22, 80 and 443, the next only 22 — were compared, and
  ports 80 and 443 were reported as having closed when they had simply not been
  scanned. Every scan now stores a coverage signature (its normalized port set
  and discovery mode), and comparison requires it to match.
- **Device identity no longer crosses networks.** Identity was global, so two
  client sites both using `192.168.1.0/24` could merge into one device when
  neither observation had a usable MAC address — mixing names, notes, status,
  first-seen dates and history between unrelated clients. Every scan and device
  now belongs to a network scope, matching never crosses one, and the default
  gateway's MAC address is used where available to tell identical private
  subnets apart. Networks can be named in Settings.
- **Ambiguous hostnames no longer merge devices.** A generic name such as
  `printer`, `router` or `localhost`, with no manufacturer to disambiguate it, is
  no longer treated as an identity; those observations stay separate devices.
- **Exporting a scan from History exports that scan.** The export opened the
  scan into the view and then exported whatever the table held, but React state
  does not update synchronously — so it could write the previously displayed
  scan, or only the rows matching the current filter, under a filename naming the
  scan you asked for. The export now fetches the requested scan and writes
  exactly its contents, leaving the current view, selection and filters
  untouched. Each history row offers CSV, JSON and XML rather than silently
  forcing CSV.
- **Device notes load after a device changes address.** Drafts were keyed by IP,
  so notes failed to appear when a device moved or when an older observation was
  opened, and half-typed text could follow the wrong device. Drafts are keyed by
  the persistent device instead. Typing survives incoming scan updates and
  renames, and a failed save no longer discards what you wrote.
- **The recommended profile is the profile that runs.** The first-run screen's
  Scan button submitted whatever profile happened to be selected while
  describing a different one. It now applies the recommendation it displays.

### Changed

- **The scan event channel is bounded.** Events crossed from the scanner to the
  interface on an unbounded queue, so a wide scan whose results the window could
  not consume fast enough grew memory without limit. The channel is now bounded:
  discovery, removal and completion events apply backpressure, progress updates
  are droppable, and neither a closed window nor a stalled consumer can wedge or
  deadlock a scan.
- **Content Security Policy is enabled.** The application shipped with CSP
  disabled. Production now allows scripts and styles only from the application
  bundle, connections only to Tauri IPC and — when you opt in — the two named
  public-IP providers, and blocks objects and frames outright. No remote scripts
  and no `eval`.
- **Application permissions are narrower.** The webview holds only what it uses:
  IPC, events, the save dialog, and the updater with its restart. It can no
  longer ask the system to open a URL; external links go through fixed, trusted
  destinations or a validated address. Export writing accepts only absolute paths
  ending in `.csv`, `.json` or `.xml`.
- **Native window decorations are explicit** in the window configuration rather
  than relied upon as a default, with the reasoning and a packaged-build
  checklist recorded in `docs/WINDOW-CHROME.md`.
- History rows show which network a scan belongs to, and the scan's coverage.

### Added

- **Regression tests for every corrected issue**: 131 Rust tests and 123
  TypeScript tests, including cancellation at each scan phase driven by
  deterministic checkpoints rather than timing, bounded-channel behaviour under
  a slow or vanished consumer, coverage-signature compatibility, cross-scope
  identity collisions, and migrations exercised against real v1.6.4 and v1.7.0
  database fixtures.
- **The browser verification suite runs in CI**, with Playwright's Chromium
  installed on the runner, and covers the behaviour this release fixes.

## [1.7.0] - 2026-07-30

ArcScan becomes a network inventory rather than a one-time scanner. Devices now
persist across scans with your own names and notes, results stream in while the
scan runs, and every scan reports what changed since the last one.

Install over 1.6.4 without deleting anything: the database migrates in place and
keeps every scan it already held.

### Added

- **Persistent device inventory.** Devices are matched across scans by MAC
  address, then hostname plus manufacturer, then IP address. A DHCP lease change
  is now reported as a device that moved rather than as one new device plus one
  missing device.
- **Change detection.** After each scan, ArcScan reports new devices, devices
  that returned, missing devices, address changes, hostname and manufacturer
  changes, operating-system changes, and ports that opened or closed. Comparison
  is against the most recent earlier scan of the same target *and* profile.
- **Live results.** Devices appear in the table while the scan is still running
  and fill in as more is learned. The status bar shows devices found, addresses
  checked, the current phase and elapsed time.
- **Scan profiles.** Quick LAN, Reliable LAN, Full TCP, Remote subnet and Custom.
  Named profiles pin their own settings so their scans stay comparable over time.
- **Device names, status and notes**, stored with the device in the database
  rather than in browser storage, so they survive a reinstall. Names follow a
  device across address changes.
- **Device detail panel** with both latency measurements, previous addresses,
  recent changes, the device's own scan history, and actions enabled by the
  services that are actually open.
- **Comparison view** separating added, missing and changed devices, with
  field-level differences.
- **History** as a timeline with per-scan change counts, and per-scan compare,
  export and delete.
- **Settings** for theme, default profile, ports, concurrency limits, row
  density, visible columns, history retention, notifications and reduced motion.
- **Keyboard shortcuts**: Enter to scan, Escape with a defined precedence,
  Ctrl/Cmd + F, E, R and L, and arrow-key navigation in the results table.
- **Scan workload preview.** The command bar shows how many addresses and ports a
  scan covers before it starts, and warns on large ones.
- **A TypeScript test suite** (Vitest, 100 tests) and two browser-driven
  verification scripts for the application and the website.
- **Dependency auditing** in CI, plus `cargo fmt --check` and a check that the
  version is consistent across every file that carries it.
- A `LICENSE` file. The project has always been MIT; the file was missing.

### Changed

- **Scanner concurrency is properly bounded.** Host concurrency alone was
  limited while each host fanned out to every selected port at once, so 64 hosts
  across 2,048 ports meant over 130,000 simultaneous connection attempts. There
  are now three independent limits: hosts (64), TCP probes across the whole scan
  (256), and `ping` child processes (32).
- **Ports are validated in Rust.** De-duplication, range checking and the 2,048
  port ceiling are enforced by the backend regardless of what the interface
  sends. An oversized range is refused with an explanation rather than truncated.
- **Scans are refused by workload**, not by address count alone: addresses
  multiplied by ports, above four million attempts, with the arithmetic shown.
- **"Ping" is now "Response".** ICMP round-trip time and TCP connection time are
  measured and stored separately, and ICMP latency is parsed from what `ping`
  itself reports rather than timing the child process.
- **The public IP lookup is opt-in.** It no longer runs at startup. It is off by
  default, names the services it contacts, and keeps the result for the session
  only.
- **A complete interface redesign** built on design tokens, with equal attention
  to the light and dark themes, a denser results table, width-aware columns and a
  consistent notification system that explains failures in plain language.
- **A complete website rebuild** with the current product screenshots, a change
  detection section, a capability comparison, per-platform download cards and a
  privacy page.
- **The version has one source of truth**, `package.json`, propagated by
  `npm run sync-version` and checked by CI.
- Exports gain name, status, ICMP and TCP columns. The 1.6 columns are unchanged
  and in the same order, so existing spreadsheets and scripts keep working.

### Fixed

- **Remote scans no longer run a pointless ARP re-prime pass.** Any non-empty ARP
  cache was treated as proof that a target was local, and every machine has a
  gateway entry, so every routed scan did extra work and had local-segment
  liveness rules applied to it. A target is local only when it overlaps a
  detected local subnet or when a scanned address has a real, non-proxy ARP
  entry.
- **A late Stop can no longer cancel the next scan.** Cancellation is scoped to a
  scan id.
- **Events from a winding-down scan can no longer appear in a newer one.** Every
  event carries a scan id and is checked against the scan being displayed.
- **New-device detection no longer compares by IP address** against the last scan
  of any network.
- Devices with no name sorted lexicographically by address, putting 10.0.0.12
  before 10.0.0.4.
- Ports with no known service rendered as `3000 · 3000`, and the service list had
  no separator between entries.
- Muted text failed WCAG AA contrast at 13px in both themes.
- The website disabled pinch zoom with `user-scalable=no`.
- An export filename could collapse to `arcscan-_`.
- Removed an unused component that referenced a colour that no longer existed.

### Migration from 1.6.4

The database migrates in place on first launch. Every scan and observation is
kept, the device inventory is built from the observations already on disk (oldest
first, so first-seen dates are truthful), scan targets are normalised so older
scans become comparable, and device labels from 1.6's browser storage are
imported. Migrations are idempotent.

## [1.6.4] and earlier

See the [release history](https://github.com/kingnazz/ArcScan/releases).
