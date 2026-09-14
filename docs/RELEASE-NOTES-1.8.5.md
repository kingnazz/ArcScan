# ArcScan 1.8.5

ArcScan 1.8.5 combines a direct ArcAtlas Discovery handoff with a reliability fix
for local network discovery on macOS.

## ArcAtlas direct handoff

- Choose one Inventory network and explicitly send its observed inventory to a
  configured ArcAtlas Discovery inbox. Nothing is uploaded merely because a scan
  completes.
- Installed ArcScan stores the connection token in the operating system credential
  store. Portable keeps it in process memory for the current session only.
- Retry uses a stable handoff identifier after uncertain failures so ArcAtlas can
  return the existing run instead of creating a duplicate.
- Pasting the full ArcAtlas machine endpoint is normalized to the server URL before
  validation, and the app exposes clearer version/build identity.

## macOS scan reliability

- A host that positively answers ICMP or TCP now remains in the final result even
  if macOS drops or omits its ARP entry before final enrichment.
- A present proxy-ARP entry is still rejected, preserving protection against router
  or access-point false positives.
- Legitimate ARP-only quiet devices are still retained.
- Regression tests cover all three paths so streamed results and saved history stay
  consistent.

## Verification

The macOS fix passed Rust formatting, unit tests, Clippy, Portable-edition tests and
Clippy, frontend tests/build, browser verification, and the universal macOS installer
build before merge. The final 1.8.5 release is rebuilt and signed from the release
commit by GitHub Actions.

## Upgrade

Installed ArcScan can use the normal in-app updater. Windows Portable users should
finish the current session, export anything they want to retain, close ArcScan, and
download the new Portable ZIP.
