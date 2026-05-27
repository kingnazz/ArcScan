# ArcScan

**Authorized LAN discovery and client reporting for MSPs.**

ArcScan is a polished Windows-first desktop app for safe, read-only network
inventory — conceptually similar to Advanced IP Scanner, but focused on
authorized assessment and clean client reporting. It discovers live hosts,
identifies services, and keeps a searchable scan history.

> ⚠️ **Authorized use only.** Only scan networks you own or have explicit
> written authorization to assess. Unauthorized network scanning may be
> illegal in your jurisdiction. ArcScan performs **read-only host discovery**
> only — it never attempts credential, brute-force, vulnerability, or
> exploitation activity. By default it refuses to scan anything outside the
> private RFC1918 ranges (`10/8`, `172.16/12`, `192.168/16`).

---

## Features

- **Flexible targets** — CIDR (`192.168.1.0/24`), dashed ranges
  (`10.0.0.1-10.0.0.50` or `10.0.0.1-50`), or a single address.
- **Hybrid discovery** — ICMP echo (via the OS `ping`, so no elevated
  raw-socket privileges are needed) with a parallel TCP probe of common
  service ports (`22, 80, 443, 445, 3389, 8080`). A host counts as *up* if it
  answers ICMP, accepts a TCP connection, or actively refuses one.
- **Rich host table** — IP, hostname (reverse DNS), MAC + vendor (from the OS
  ARP cache, same-subnet only), open ports, response time, and last-seen, with
  live filtering and column sorting.
- **One-click actions** — copy IP, open the web interface, launch RDP
  (`mstsc`), or open an SSH session.
- **Dashboard** — devices found, unknown devices, open RDP, open SMB, and new
  devices since the last scan.
- **Scan history** — every scan is saved to SQLite and can be reopened or
  deleted from the sidebar.
- **CSV export** — export the current result set for client reporting.
- **Premium dark UI** — built with Tailwind, no toy/template look.

---

## Tech stack

| Layer      | Choice                                   |
| ---------- | ---------------------------------------- |
| Shell      | [Tauri 2](https://tauri.app)             |
| Frontend   | React 18 + TypeScript + Vite             |
| Styling    | Tailwind CSS 3                           |
| Backend    | Rust (async via Tokio)                   |
| Storage    | SQLite (via `rusqlite`, bundled)         |

### Project layout

```
ArcScan/
├── src/                     # React + TypeScript frontend
│   ├── components/          # Dashboard, ScanControls, ResultsTable, …
│   ├── hooks/useScan.ts     # Scan lifecycle + event buffering
│   ├── lib/                 # api (Tauri bridge), ip parsing, csv, mock
│   └── types.ts             # Shared types mirroring the Rust structs
├── src-tauri/               # Rust backend
│   └── src/
│       ├── scanner.rs       # Discovery engine (ICMP + TCP, ARP, rDNS)
│       ├── db.rs            # SQLite schema + persistence
│       ├── oui.rs           # Embedded MAC vendor lookup
│       ├── commands.rs      # Tauri command handlers + launch actions
│       └── lib.rs / main.rs # App wiring
└── src-tauri/tauri.conf.json
```

### Backend command surface

| Command           | Purpose                                          |
| ----------------- | ------------------------------------------------ |
| `scan_network`    | Run a scan; streams `scan://progress` / `scan://host` events |
| `cancel_scan`     | Cooperatively stop the running scan              |
| `list_scans`      | Saved scan summaries (newest first)              |
| `get_scan_hosts`  | Hosts for a saved scan                           |
| `delete_scan`     | Remove a saved scan                              |
| `launch_action`   | Open web / RDP / SSH for a host                  |
| `write_text_file` | Persist exported CSV to a chosen path            |

The scan-history database lives in the OS app-data directory, e.g. on Windows:
`%APPDATA%\com.arcscan.app\arcscan.db`.

---

## Development

### Prerequisites

- **Node.js** 18+ and **npm**
- **Rust** (stable) via [rustup](https://rustup.rs)
- Platform toolchain for Tauri 2 — see the
  [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/):
  - **Windows:** Microsoft C++ Build Tools + the WebView2 runtime (preinstalled
    on Windows 11; the installer bundles it otherwise).
  - **Linux (dev/build host):** `libwebkit2gtk-4.1-dev`, `libsoup-3.0-dev`,
    `libgtk-3-dev`, `librsvg2-dev`, plus the usual `build-essential` tooling.
  - **macOS:** Xcode command-line tools.

### Browser preview (mock data — no Rust needed)

The UI runs fully in a browser against a built-in mock scanner, which is handy
for frontend work:

```bash
npm install
npm run dev          # http://localhost:1420
```

### Desktop app (real scanning)

```bash
npm install
npm run tauri:dev    # builds the Rust backend and opens the desktop window
```

### Useful scripts

```bash
npm run typecheck    # tsc --noEmit
npm run build        # type-check + production frontend bundle
cargo check --manifest-path src-tauri/Cargo.toml
```

---

## Packaging

ArcScan uses Tauri's bundler. Production builds run `npm run build` for the
frontend automatically (configured via `beforeBuildCommand`).

### Windows (primary target)

```bash
npm install
npm run tauri:build
```

Artifacts are written to `src-tauri/target/release/bundle/`:

- `msi/ArcScan_0.1.0_x64_en-US.msi` — WiX MSI installer
- `nsis/ArcScan_0.1.0_x64-setup.exe` — NSIS setup executable
- `src-tauri/target/release/arcscan.exe` — the raw executable

To produce only one installer type:

```bash
npm run tauri:build -- --bundles msi      # or: nsis
```

> **Cross-compiling for Windows from Linux/macOS is not recommended** for Tauri
> apps because of the WebView2/MSVC toolchain. Build Windows artifacts on
> Windows (or a Windows CI runner / VM).

### macOS / Linux

The same `npm run tauri:build` produces `.dmg`/`.app` on macOS and
`.deb`/`.AppImage`/`.rpm` on Linux. RDP launching on non-Windows hosts uses
FreeRDP (`xfreerdp`) if it is installed.

### App icons

Icons are generated programmatically (no external assets) — regenerate with:

```bash
python3 src-tauri/icons/generate_icons.py
```

This writes `32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.png`, the
Windows Store logos, and a multi-resolution `icon.ico`.

---

## Security & scope

- **RFC1918 guard.** Public targets are rejected unless the operator explicitly
  enables *Allow public range*; the backend re-validates this independently of
  the UI.
- **Authorization acknowledgement** is required before any scan can start.
- **Read-only by design.** No exploit code, brute forcing, credential attacks,
  or vulnerability exploitation — discovery and service detection only.
- **No privilege escalation.** ICMP is performed via the OS `ping` utility, so
  ArcScan does not need raw-socket / administrator rights to function.
- **Input is validated** on both sides; launch actions only accept bare IPv4
  addresses to avoid argument injection.
