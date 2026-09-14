# ArcScan 1.8.6

ArcScan 1.8.6 is a focused reliability and polish release for macOS local-network discovery and the ArcAtlas handoff UI.

## Fixed

### macOS local discovery no longer loses most devices after a scan

ArcScan now keeps macOS neighbor-table and ICMP discovery fully numeric. This prevents reverse-DNS lookups from consuming the scanner's short discovery timeouts on populated local networks.

- Reads the macOS/BSD ARP cache with `arp -n -a` so the operating system does not attempt a hostname lookup for every neighbor before ArcScan can parse the table.
- Runs macOS `ping` in numeric mode as well.
- Uses the macOS per-reply timeout and exits after the first valid reply.
- Keeps the 1.8.5 behavior that preserves a host with positive ICMP/TCP evidence when a neighbor-cache entry is temporarily missing.
- Keeps proxy-ARP protection, so a router or access point answering on behalf of many addresses does not turn the entire subnet into false-positive devices.

This specifically addresses the failure mode where Inventory could know about many devices while a fresh macOS scan completed with only a small handful of results.

## Improved

### Cleaner ArcAtlas handoff controls

The Inventory toolbar no longer shows a separate **ArcAtlas** button beside **Send to ArcAtlas**. The send action is now the single primary ArcAtlas control, with connection management available from the send flow when needed.

## Verification

The 1.8.6 release candidate is verified with the project CI suite, including Rust tests and Clippy, Portable-edition checks, frontend typechecking/tests/build, browser UI verification, Content Security Policy checks, marketing-site verification, `npm audit`, `cargo audit`, and cross-platform installer builds.
