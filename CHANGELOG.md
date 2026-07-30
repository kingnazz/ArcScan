# Changelog

All notable changes to ArcScan. This project follows
[semantic versioning](https://semver.org/spec/v2.0.0.html).

## [1.7.0] - unreleased

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
