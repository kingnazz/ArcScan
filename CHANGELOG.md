# Changelog

All notable changes to ArcScan. This project follows
[semantic versioning](https://semver.org/spec/v2.0.0.html).

## [1.7.1] - unreleased

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
