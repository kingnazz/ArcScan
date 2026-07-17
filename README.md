<div align="center">

# ArcScan

**Authorized LAN discovery & client reporting for MSPs**

A polished, safe, read-only network inventory tool — conceptually similar to
Advanced IP Scanner, but built for managed service providers who need fast,
authorized discovery and clean client reporting.

Tauri 2 · React + TypeScript · Tailwind CSS · Rust (Tokio) · SQLite

</div>

---

> ⚠️ **Authorized use only.** ArcScan performs read-only discovery. Only scan
> networks you own or have **explicit written authorization** to assess.
> Unauthorized network scanning may be illegal in your jurisdiction. ArcScan
> contains no exploit, brute-force, credential-attack, or vulnerability-
> exploitation code, and never will.

## Features

- **Flexible targets** — CIDR (`192.168.1.0/24`), dashed ranges
  (`10.0.0.1-10.0.0.50` or the short `10.0.0.1-50`), and single IPs.
- **Robust liveness detection** — ICMP echo via the OS `ping` binary (no
  raw-socket/administrator privileges required), with a TCP fallback on ports
  22, 80, 443, 445, 3389, and 8080. A host counts as **up** if it answers
  ICMP, accepts a TCP connection, *or* actively refuses one (RST) — all three
  prove liveness.
- **Fast, modern results table** — IP, hostname, MAC, vendor, open ports,
  response time, and last-seen, with column sorting and instant
  filtering/search.
- **Per-host actions** — copy IP, open web interface, open RDP, open SSH, and
  export the whole result set to CSV via a native save dialog.
- **Scan history** — every scan is saved to a local SQLite database and is
  browsable, re-openable, and deletable.
- **Dashboard** — total devices, unknown devices, open RDP count, open SMB
  count, and **new devices since the last scan**.
- **Full IEEE OUI vendor registry** — ~39,000 MA-L prefixes embedded for real
  vendor identification, loaded once into an in-memory map for O(1) lookups.
- **Tunable performance** — per-host timeout and max concurrency are exposed as
  advanced options (defaults: 128 concurrent probes, 600 ms timeout).

## Safety & scope (non-negotiable)

These constraints are enforced in the Rust backend, **independently of the UI**
(the frontend cannot bypass them):

1. **Private ranges only by default.** Only RFC1918 ranges (`10.0.0.0/8`,
   `172.16.0.0/12`, `192.168.0.0/16`) are scanned unless the operator
   explicitly enables the **Allow public range** toggle. The backend
   re-validates every target address regardless of what the UI sends.
2. **Explicit authorization required.** Every scan requires the *"I am
   authorized to scan this network"* acknowledgement, and a persistent warning
   is always visible.
3. **Read-only discovery only.** No exploit, brute-force, credential, or
   vulnerability-exploitation logic exists in the codebase.
4. **Injection-safe launches.** Before shelling out to launch RDP/SSH/a
   browser, the backend accepts only a well-formed bare IPv4 address, so no
   argument injection is possible.

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
│       ├── ipparse.rs        Target parsing + RFC1918 validation (+ unit tests)
│       ├── scanner.rs        Ping / TCP probes, ARP, concurrent DNS (+ tests)
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

## Packaging

Build native installers locally:

```bash
npm run tauri:build            # MSI + NSIS for the host architecture
```

Artifacts land in `src-tauri/target/<target>/release/bundle/` (`msi/*.msi` and
`nsis/*-setup.exe`).

### CI / releases

`.github/workflows/build.yml` builds a matrix of both Windows CPU
architectures on `windows-latest` — the hosted runner ships the MSVC ARM64
cross-compile tools, so no dedicated ARM runner is needed:

| Target | Artifact |
| --- | --- |
| `x86_64-pc-windows-msvc` | `ArcScan-windows-x64` (MSI + NSIS `.exe`) |
| `aarch64-pc-windows-msvc` | `ArcScan-windows-arm64` (MSI + NSIS `.exe`) |

Each is uploaded as a separate downloadable artifact. Pushing a `v*` tag also
drafts a GitHub Release with all four installers attached.

> **Version bumps matter.** Windows Installer treats installing the *same*
> version as a no-op and will **not** replace an installed binary. Any change
> meant to reach an already-installed machine must bump the version in all
> three of `package.json`, `src-tauri/Cargo.toml`, and
> `src-tauri/tauri.conf.json` (keep them in sync).

## Regenerating assets

Both are one-time generation steps; the outputs are committed to the repo so
builds are reproducible offline.

```bash
# App icon set (PNG sizes + Windows .ico) — pure Python standard library
python3 scripts/generate_icon.py

# Vendor table from the live IEEE MA-L registry (downloads ~4 MB CSV)
python3 scripts/generate_oui.py
# or, from a local copy:
python3 scripts/generate_oui.py path/to/oui.csv
```

## License

MIT
