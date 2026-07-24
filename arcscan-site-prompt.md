# Build prompt — ArcScan download site

Build a small, fast, **static marketing + download site** for ArcScan, a
lightweight desktop network & port scanner (a Tauri app). The site's job is to
explain what ArcScan is and let visitors **download the right installer for
their OS in one click**. It must always offer the newest published release
without anyone editing HTML.

## Context you can rely on

- **Repo (public):** `kingnazz/ArcScan` — the desktop app lives here.
- **Releases:** each version is published to this repo's GitHub Releases by the
  `Publish Release` workflow. Assets are named like:
  - Windows x64:  `ArcScan_<version>_x64-setup.exe`
  - Windows ARM64: `ArcScan_<version>_arm64-setup.exe`
  - macOS (universal): `ArcScan_<version>_universal.dmg`
  - Plus `.sig` files and `latest.json` (updater manifest) — ignore these on the site.
- **Stable "latest" permalinks** always resolve to the newest release, e.g.
  `https://github.com/kingnazz/ArcScan/releases/latest/download/<asset-name>`.
  Note the asset name still contains the version, so prefer the GitHub Releases
  **API** to discover exact asset URLs rather than hardcoding names.
- **Brand:** teal/cyan accent (primary ~`#0898b8`, lighter `#12b8db`), clean and
  modern, **light theme by default** with an optional dark mode. The app icon is
  in the repo at `src-tauri/icons/128x128@2x.png` (and `icon.png`) — reuse it as
  the site logo/favicon so branding matches the app.
- ArcScan is Windows-first (x64 + ARM64) and also ships a macOS universal build.

## Tech & hosting

- **Static site, no backend.** Plain HTML + CSS + a small vanilla-JS file is
  ideal; a tiny Vite setup is fine if it stays dependency-light. No React/Next.
- Put the site in a **`/site` directory** in the ArcScan repo so it ships
  alongside the app without disturbing the desktop build.
- **Deploy via GitHub Pages** using a GitHub Actions workflow
  (`.github/workflows/pages.yml`) that publishes `/site` on push to `main`.
  Everything must be self-contained (inline or local CSS/JS, local images) — no
  external CDNs or trackers.
- Fully responsive (mobile → desktop), accessible (semantic HTML, keyboard-
  focusable controls, sufficient contrast), and fast (no heavy frameworks).

## Core behavior — dynamic downloads (the important part)

On page load, fetch the latest release from the GitHub REST API:
`GET https://api.github.com/repos/kingnazz/ArcScan/releases/latest`
(unauthenticated; handle rate-limit/errors gracefully). From the response:

1. Read `tag_name` (e.g. `v1.6.1`) and show it as the current version and
   release date somewhere near the download buttons.
2. Map `assets[]` by filename to three download targets:
   - `*x64-setup.exe`  → **Windows (x64)**
   - `*arm64-setup.exe` → **Windows (ARM64)**
   - `*universal.dmg`  → **macOS**
   Use each asset's `browser_download_url`.
3. **Detect the visitor's OS/arch** (`navigator.userAgent` /
   `navigator.userAgentData` where available) and present the best-matching
   download as the **big primary button** ("Download for Windows"), with the
   other platforms/architectures shown as smaller secondary links ("Other
   downloads: Windows ARM64 · macOS").
4. **Fallback if the API fails or is rate-limited:** link the primary button to
   the stable release page `https://github.com/kingnazz/ArcScan/releases/latest`
   and still render the secondary platform links pointing there. The site must
   never show a dead/blank download button.

## Page content

Single landing page (one scrolling page is fine), in this order:

1. **Header** — ArcScan logo + name, nav anchors (Features, Screenshots, FAQ),
   a "Download" button, and a light/dark toggle.
2. **Hero** — one-line value prop ("A fast, lightweight network & port scanner
   for Windows and macOS."), a short supporting sentence, the **primary download
   button** (OS-detected) + secondary platform links, and the current version /
   "Free · Open source". Optionally a hero screenshot of the app.
3. **Features** — a grid of ~6 concise cards drawn from what ArcScan actually
   does: auto-detects your subnet; discovers live hosts via ICMP + TCP with a
   stable ARP-verified LAN sweep; shows IP / hostname / MAC / manufacturer / OS
   guess / open ports / ping; per-host actions (open web, RDP, SSH, SMB shares,
   Wake-on-LAN, copy); sortable/filterable results; saved/known devices; scan
   history; export to CSV / JSON / XML; built-in auto-update. Keep copy honest —
   this is **read-only discovery**, not an exploitation tool.
4. **Screenshots** — 2–3 images of the app (light + dark). Leave clearly-named
   placeholders (`site/assets/screenshot-light.png`, `-dark.png`) and reference
   them; I'll drop real captures in. Use `loading="lazy"`.
5. **How it works / install notes** — 3 short steps (download → run installer →
   scan your network). Note Windows SmartScreen may warn on a new publisher and
   how to proceed; note macOS Gatekeeper right-click-Open the first time. Mention
   the app updates itself once installed.
6. **FAQ** — a few Q&As: Is it free? Which OSes/architectures? Does it need
   admin? Is my data sent anywhere? (No — scans are local, nothing is uploaded.)
   How do updates work? Where's the source? (link the repo.)
7. **Footer** — links to the GitHub repo, Releases, and Issues; copyright;
   "Made with ArcScan" style note. No fake company/legal claims.

## Design details

- Use the brand teal as the accent on buttons/links/highlights; neutral
  gray-blue surfaces; generous whitespace; rounded-md corners; subtle shadows.
- Light theme is the default; dark theme via `prefers-color-scheme` **and** a
  manual toggle persisted to `localStorage`.
- System font stack (no downloaded web fonts) for speed and privacy.
- Buttons show a small OS glyph (Windows / Apple) and the file size if the API
  provides it (`asset.size`).

## Constraints & polish

- **No secrets, no tracking, no external scripts.** Everything served from the
  repo. Do not collect analytics or emails.
- Don't invent testimonials, download counts, awards, or a company identity.
- Keep total page weight small; inline critical CSS is fine.
- Include a short `site/README.md` explaining how to run it locally (just open
  `index.html`, or `python3 -m http.server` in `/site`) and how the Pages
  deploy works.
- After building: verify the page renders, the OS detection picks a sensible
  default, and the API-driven buttons populate (or fall back cleanly when the
  API is blocked). Commit to a feature branch and open a draft PR to `main`;
  once the Pages workflow is added, note the resulting site URL
  (`https://kingnazz.github.io/ArcScan/`) in the PR.

## Acceptance criteria

- Visiting the site on Windows shows a working "Download for Windows (x64)"
  button that fetches the current release's `x64-setup.exe`; ARM64 and macOS are
  one click away.
- The version label matches the newest GitHub Release with no manual edits.
- With the GitHub API blocked, every download control still routes to
  `releases/latest`.
- Looks clean and on-brand in both light and dark, on phone and desktop.
- Ships as `/site` + a Pages deploy workflow, no impact on the desktop app build.
