<div align="center">

# ArcScan

**A fast, lightweight network & port scanner**

A polished, read-only network scanner in the spirit of Angry IP Scanner,
Advanced IP Scanner, and Advanced Port Scanner — discover live hosts, identify
vendors and open services, and export the results. Light and dark themes.

Tauri 2 · React + TypeScript · Tailwind CSS · Rust (Tokio) · SQLite

</div>

---

## Features

- **Auto-detects your network** — on launch ArcScan fills the target with your
  own subnet (and shows this device's IP); a **Detect** button re-runs it.
- **Flexible targets** — CIDR (`192.168.1.0/24`), dashed ranges
  (`10.0.0.1-10.0.0.50` or the short `10.0.0.1-50`), and single IPs.
- **Robust liveness detection** — ICMP echo via the OS `ping` binary (no
  raw-socket/administrator privileges required), plus a TCP fallback across a
  curated set of common ports (FTP, SSH, Telnet, DNS, HTTP/S, SMB, RDP, VNC,
  and more). A host counts as **up** if it answers ICMP, accepts a TCP
  connection, actively refuses one (RST), **or appears in the ARP cache** — so
  devices that silently drop pings/probes (phones, IoT, printers, firewalled
  hosts) are still found on the local segment, the way Advanced IP / Angry IP
  do it.
- **Port & service detection** — the default port set fingerprints most hosts
  at a glance; enter single ports **and ranges** (`1-1024`, `80,443,8000-8100`)
  in the advanced options.
- **OS guess** — an Angry-IP-style TTL fetcher labels each host
  Windows / Linux-Unix-macOS / network device.
- **Fast, modern results table** — IP, hostname, MAC, vendor, OS, open ports,
  response time, and last-seen, with column sorting and instant
  filtering/search.
- **Per-host actions** — copy IP, open web interface, open shared folders
  (SMB), open RDP, open SSH, and send a **Wake-on-LAN** magic packet.
- **Multi-format export** — export the whole result set to **CSV, JSON, or
  XML** via a native save dialog.
- **Scan history** — every scan is saved to a local SQLite database and is
  browsable, re-openable, and deletable.
- **Dashboard** — total devices, unknown devices, open RDP count, open SMB
  count, and **new devices since the last scan**.
- **Light & dark themes** — a clean modern light theme by default, with a
  one-click dark mode.
- **Full IEEE OUI vendor registry** — ~39,000 MA-L prefixes embedded for real
  vendor identification, loaded once into an in-memory map for O(1) lookups.
- **Tunable performance** — per-host timeout and max concurrency are exposed as
  advanced options (defaults: 128 concurrent probes, 600 ms timeout).

## How it works

ArcScan is **read-only discovery only** — it sends ICMP pings and attempts TCP
connections to detect live hosts and open ports. There is no exploit,
brute-force, credential, or vulnerability-exploitation logic in the codebase.
When launching RDP/SSH/a browser for a host, the backend accepts only a
well-formed bare IPv4 address, so there's no room for argument injection.

As with any network scanner (nmap, Angry IP Scanner, …), only scan networks you
own or have permission to scan.

## Architecture

```
ArcScan/
├── src/                      React + TypeScript frontend
│   ├── lib/
│   │   ├── api.ts            Single API surface — detects Tauri, falls back to mock
│   │   ├── mock.ts           Pure-TS mock scanner (runs in a plain browser)
│   │   └── format.ts         Formatting + service helpers
│   ├── components/           Dashboard, ScanControls, HostsTable, ScanHistory, …
│   ├── types.ts              Types mirrored from the Rust serde structs
│   └── App.tsx               Orchestration + dashboard stats
├── src-tauri/                Rust backend
│   └── src/
│       ├── ipparse.rs        Target parsing + host-count guard (+ unit tests)
│       ├── netinfo.rs        Local interface/subnet detection (auto-fill)
│       ├── scanner.rs        Ping / TCP probes, TTL/OS, ARP, DNS (+ tests)
│       ├── oui.rs            Embedded IEEE OUI vendor lookup
│       ├── db.rs             SQLite persistence (bundled rusqlite)
│       ├── commands.rs       Tauri command surface + CSV + launch helpers
│       └── oui_data.tsv      Compact embedded vendor table (generated)
├── scripts/
│   ├── generate_icon.py      Pure-stdlib PNG/ICO icon generator
│   └── generate_oui.py       Compresses the IEEE OUI CSV into oui_data.tsv
└── .github/workflows/        CI (checks) + Windows x64/ARM64 installer builds
```

The frontend talks to exactly one module (`src/lib/api.ts`). When running
inside Tauri it calls the native Rust commands; in a plain browser it
transparently falls back to `src/lib/mock.ts`, so the entire UI can be
developed and demoed with **no native backend**.

## Development

### Prerequisites

- **Node.js** 20+
- **Rust** (stable) + the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/)
  for your platform. On Windows that means the MSVC build tools and WebView2
  (preinstalled on Windows 11).

### Frontend-only (browser, mock data)

```bash
npm install
npm run dev        # http://localhost:1420 — full UI with the mock scanner
```

### Full desktop app (native scanning)

```bash
npm install
npm run tauri:dev  # launches the native window with the real Rust backend
```

### Useful commands

```bash
npm run typecheck              # TypeScript
npm run build                  # frontend production build
cd src-tauri && cargo test     # Rust unit tests (IP parsing, ARP, OUI)
cd src-tauri && cargo clippy --all-targets -- -D warnings
```

## Platform support

ArcScan is Windows-first but fully cross-platform. It runs on:

- **Windows** — Intel/AMD (x64) **and** Windows-on-ARM (ARM64) devices
- **macOS** — Apple Silicon (arm64) **and** Intel (x86_64), shipped as a single
  universal binary

The scanner adapts its `ping`/`arp` invocations per OS, suppresses child console
windows on Windows (`CREATE_NO_WINDOW`), and adapts the RDP/SSH launch helpers
(e.g. macOS RDP via the `rdp://` scheme, SSH via a Terminal session).

## Packaging

Build native installers locally:

```bash
npm run tauri:build                                  # host-arch installer(s)
npm run tauri:build -- --target universal-apple-darwin --bundles dmg   # macOS universal DMG
```

Artifacts land in `src-tauri/target/<target>/release/bundle/` — `msi/*.msi` and
`nsis/*-setup.exe` on Windows, `dmg/*.dmg` on macOS.

### CI / releases

`.github/workflows/build.yml` builds all supported platforms in one matrix.
`windows-latest` ships the MSVC ARM64 cross tools (so no dedicated ARM runner is
needed), and `macos-latest` produces a single universal binary covering both Mac
architectures:

| Runner | Target | Artifact |
| --- | --- | --- |
| `windows-latest` | `x86_64-pc-windows-msvc` | `ArcScan-windows-x64` (MSI + NSIS `.exe`) |
| `windows-latest` | `aarch64-pc-windows-msvc` | `ArcScan-windows-arm64` (MSI + NSIS `.exe`) |
| `macos-latest` | `universal-apple-darwin` | `ArcScan-macos-universal` (`.dmg`) |

Each is uploaded as a separate downloadable artifact. Pushing a `v*` tag also
drafts a GitHub Release with every installer attached.

> **Version bumps matter.** Windows Installer treats installing the *same*
> version as a no-op and will **not** replace an installed binary. Any change
> meant to reach an already-installed machine must bump the version in all
> three of `package.json`, `src-tauri/Cargo.toml`, and
> `src-tauri/tauri.conf.json` (keep them in sync).

## Regenerating assets

Both are one-time generation steps; the outputs are committed to the repo so
builds are reproducible offline.

```bash
# App icon set (PNG sizes + Windows .ico + macOS .icns) — pure Python stdlib
python3 scripts/generate_icon.py

# Vendor table from the live IEEE MA-L registry (downloads ~4 MB CSV)
python3 scripts/generate_oui.py
# or, from a local copy:
python3 scripts/generate_oui.py path/to/oui.csv
```

## License

MIT
