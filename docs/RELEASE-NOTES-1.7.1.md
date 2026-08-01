# ArcScan v1.7.1

A correctness, reliability and security release. v1.7.0 made ArcScan a network
inventory; v1.7.1 makes it one you can trust with real client networks. There is
no new product surface — every change below either stops ArcScan telling you
something untrue about a network, or narrows what the application is allowed to
do.

Installs over 1.6.x and 1.7.0 without losing scan history, device names, notes,
status, first-seen dates or settings.

## Reliable cancellation

Stop now stops the scan during every phase. Previously cancellation was read once,
after the address sweep, so a scan stopped during ARP settling, quiet-device
confirmation or hostname resolution kept doing that work and was then recorded as
*completed*. Cancellation is checked before each phase and during both settle
waits, which end immediately on Stop instead of running out a fixed delay, and the
final state is decided at the end of the scan. Whatever was found — including
manufacturer and hostname data already resolved — is kept.

## Correct partial-scan handling

A stopped scan has not seen its whole target, so it can no longer produce missing
devices or closed ports. Partial scans are still saved, opened and exported in
full; they are simply never compared, and never become the baseline for another
scan. A completed scan skips any newer partial scan and compares against the last
completed one. History labels them **Partial scan** and the Changes view explains
why there is nothing to show.

## Safer comparisons

Two scans are only compared when they actually looked for the same things.
Compatibility previously rested on the target and the profile name, so two Custom
scans of one subnet — the first probing 22, 80 and 443, the second only 22 — were
compared, and ports 80 and 443 were reported as closed when they had never been
scanned. Each scan now records a coverage signature covering its normalized port
set and discovery mode, and comparison requires it to match.

## Network-scoped device identity

Device identity was global. Two client sites both on `192.168.1.0/24` could merge
into a single device whenever an observation had no usable MAC address, mixing
names, notes, status, first-seen dates and history between unrelated clients.
Every scan and device now belongs to a network scope; matching, names and notes
never cross one. Where the default gateway's MAC address can be observed it is
used to tell identical private subnets apart. Networks can be named in Settings
("Head office", "Client VPN"), and history shows which network a scan belongs to.

Generic hostnames — `printer`, `router`, `localhost` and similar — no longer act
as an identity on their own, so two unrelated devices sharing a default name stay
separate.

## Correct historical exports

Exporting a scan from History exports that scan. The old flow opened the scan into
the view and then exported whatever the table held; because view updates are not
synchronous, it could write the previously displayed scan, or only the rows
matching the current filter, under a filename naming the scan you asked for. The
export now reads the requested scan directly and leaves your current view,
selection and filters untouched. Each row offers CSV, JSON and XML.

## Reliable device notes

Notes and names are now tied to the device rather than to its address, so they
load correctly after a device changes IP and when viewing older observations.
Typing survives incoming scan updates and renames, and a failed save keeps what
you wrote instead of discarding it.

## Event-stream reliability

The channel carrying live results from the scanner to the interface is bounded.
A wide scan whose results the window could not consume quickly enough previously
grew memory without limit. Discovery and completion events now apply
backpressure, progress updates are droppable, and neither a closed window nor a
stalled consumer can stall or deadlock a scan.

## Security hardening

- A restrictive Content Security Policy is enabled; it was previously disabled.
  Scripts and styles come only from the application bundle, network connections
  only from Tauri IPC and — when you opt in — the two named public-IP providers.
  No remote scripts, no `eval`, no frames.
- Application permissions are reduced to what is actually used. The interface can
  no longer ask the system to open an arbitrary URL: external links go to fixed,
  trusted destinations or to a validated address.
- Export writing accepts only absolute paths ending in `.csv`, `.json` or `.xml`.
- Launcher inputs (Web, SMB, RDP, SSH, Wake-on-LAN) have regression tests for
  argument injection, embedded schemes and shell metacharacters.

## Regression tests

Every fix above is covered: 131 Rust tests and 123 TypeScript tests, plus a
browser suite that now runs in CI. Cancellation is tested at each scan phase
using deterministic checkpoints rather than timing, and the database migration is
exercised against real v1.6.4 and v1.7.0 fixtures, including repeated runs.

## Upgrading

The database migrates in place, in a single transaction, and is safe to run
repeatedly. Nothing is deleted. Scans recorded by earlier versions that never
saved which ports they checked — Custom and Full TCP scans from v1.6 and v1.7.0 —
are kept and readable but are not compared, because their coverage is unknown and
a wrong comparison is worse than none.
