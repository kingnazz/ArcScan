# ArcScan 1.9.1

Released September 22, 2026.

ArcScan 1.9.1 is a focused topology reliability and diagnostics release.

## Highlights

- Adds an offline topology replay lab so captured, sanitized discovery evidence can be replayed through the same correlation and diagnostics engine without making network calls.
- Fixes the ArcScan to ArcAtlas VLAN contract so invalid VLAN values cannot reject an otherwise valid topology handoff.
- Treats CDP native VLAN 0 as absent, preserves only VLAN IDs 1 through 4094, and never truncates oversized SNMP values into plausible VLANs.
- Keeps valid links and VLAN facts when one neighboring VLAN fact is invalid.
- Keeps duplicate IP or MAC identity evidence unresolved instead of silently choosing one device.

## Release assets

The GitHub release workflow publishes signed updater artifacts, Windows x64 and ARM64 installers, Windows portable ZIPs, the universal macOS build, and `latest.json` for installed auto-update clients.
