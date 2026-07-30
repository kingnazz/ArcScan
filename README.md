<div align="center">

# ArcScan

**A fast, private network inventory and scanner for IT professionals**

See every device on your network, keep a local inventory of them, and know
exactly what changed since the last scan.

Tauri 2 · React + TypeScript · Tailwind CSS · Rust (Tokio) · SQLite

[Download](https://github.com/kingnazz/ArcScan/releases/latest) ·
[Website](https://kingnazz.github.io/ArcScan/) ·
[User guide](docs/USER-GUIDE.md) ·
[Privacy](https://kingnazz.github.io/ArcScan/privacy.html)

</div>

---

ArcScan discovers the devices on a network, records them in a local inventory,
and reports what is different from the previous scan. It is a desktop
application: everything runs on your own computer, there is no account, no cloud
service and no subscription, and nothing it finds is uploaded anywhere.

It is deliberately **not** a vulnerability scanner, a penetration-testing tool or
a monitoring platform. It performs read-only discovery and reports what is
reachable.

## What it does

**Know what is connected.** Every device that answers, with its IP address, MAC
address, manufacturer (from the full IEEE OUI registry), hostname, open services,
operating-system estimate, TTL and both ICMP and TCP response times.

**See what changed.** New devices, missing devices, address changes, hostname and
manufacturer changes, and ports that opened or closed since the previous scan of
the same target and profile.

**Keep an inventory.** Devices persist across scans, matched by MAC address first
so a DHCP lease change reads as a device that moved rather than a new one. Give
them names, a status and notes; all of it survives reinstalls.

**Investigate without leaving.** Open a device's web interface, SMB shares, SSH or
Remote Desktop from the inventory, or send a Wake-on-LAN packet. Only the actions
the open services support are enabled.

**Find the quiet devices.** ARP-assisted discovery finds printers, phones, access
points, cameras and firewalled hosts that ignore ICMP entirely, and a second
confirmation pass keeps results consistent between scans.

**Control the scan.** Five profiles from a quick sweep to a full port range, plus
your own ports, timeout and all three concurrency limits. The workload is shown
before the scan starts, and Stop keeps whatever was found.

## Install

Download the installer for your platform from the
[latest release](https://github.com/kingnazz/ArcScan/releases/latest).

| System | Architectures | Installer |
| --- | --- | --- |
| Windows 10 and 11 | x64, ARM64 | NSIS setup (`.exe`) |
| macOS 11 or later | Universal (Apple Silicon and Intel) | Disk image (`.dmg`) |

Scanning needs no administrator or root privileges. ArcScan is not code-signed
with a paid publisher certificate and the macOS builds are not notarised, so both
systems warn on first launch; the
[user guide](docs/USER-GUIDE.md#installation) explains what to do. SHA-256
checksums are published with every release.

## Network requests

Scanning is entirely local. Scan results, the device inventory, your names and
your notes are written to a SQLite file on your computer and are never sent
anywhere.

ArcScan makes exactly two optional outbound requests:

- **Update check**, on launch, against GitHub. It sends only the version being
  checked. Switchable off in Settings.
- **Public IP lookup**, which is **off by default** and only runs when you press
  the button. It contacts `api64.ipify.org`, then `icanhazip.com`, sends nothing
  but the request, and keeps the answer for the session only.

There is no telemetry, no analytics and no account. The
[privacy page](https://kingnazz.github.io/ArcScan/privacy.html) states all of
this precisely, including what GitHub sees when you download a release.

## Documentation

The [user guide](docs/USER-GUIDE.md) covers profiles, targets, live results, the
inventory, change detection, history, exporting, device actions, keyboard
shortcuts, where the database lives, upgrading, uninstalling, troubleshooting,
known limitations, and responsible scanning.

- [Auto-update setup](docs/AUTO_UPDATE.md)
- [Website](site/README.md)

## Development

Requires [Node 20+](https://nodejs.org) and a
[Rust toolchain](https://rustup.rs), plus
[Tauri's system dependencies](https://tauri.app/start/prerequisites/) on Linux.

```sh
npm install
npm run tauri:dev        # the desktop application
npm run dev              # the interface alone, in a browser, with demo data
```

The interface falls back to a built-in demo network when it is not running inside
Tauri, so the whole UI is developable and reviewable without a Rust build.

### Checks

```sh
npm run typecheck        # TypeScript
npm test                 # Vitest
npm run build            # production bundle
npm run check-version    # the version matches everywhere

cd src-tauri
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Both are run by [CI](.github/workflows/ci.yml) on every pull request, along with
a dependency audit.

Two browser-driven verification scripts are not in CI, because the hosted runners
have no browser. Run them locally against a preview build:

```sh
npm run build && npm run preview &
npm i --no-save playwright @axe-core/playwright
node scripts/verify-ui.mjs      # 20 checks against the application
cd site && python3 -m http.server 4174 &
node scripts/verify-site.mjs    # 22 checks against the website, including axe-core
```

`scripts/capture-screenshots.mjs` regenerates the website's product screenshots
from the real application, so no published image can show an older interface.

### Versioning

`package.json` is the single source of truth. Bump it, then:

```sh
npm run sync-version
```

which writes the same version into `Cargo.toml`, the Tauri config and the
website. CI fails if any of them drift.

## Project layout

```
src/                    React interface
  lib/                  pure logic: live merging, table, profiles, export, actions
  ui/                   design-system primitives
  components/           application surfaces
  hooks/                scanning, settings, theme, shortcuts
src-tauri/src/
  scanner.rs            discovery, probing, concurrency limits
  inventory.rs          device identity and change detection
  db.rs                 SQLite schema, migrations and queries
  ports.rs              port parsing, validation and the service table
  ipparse.rs            target parsing and normalisation
site/                   the GitHub Pages website
scripts/                version sync, screenshots, verification
```

## Contributing

Issues and pull requests are welcome. Please run the checks above before opening
one.

Contributions that add exploitation, credential guessing, brute force, payload
execution, persistence, evasion, stealth scanning or internet-wide scanning will
not be accepted. ArcScan is a network administration tool, and that is the whole
of its scope.

## License

[MIT](LICENSE).

Scan only networks you own or are authorised to inspect.
