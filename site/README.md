# ArcScan download site

A tiny, dependency-free static site that presents ArcScan and serves the newest
installer for each platform. It reads the latest GitHub Release at runtime, so it
never needs editing when a new version ships.

## Files

- `index.html` — the page (hero, features, preview, install steps, FAQ, CTA).
- `styles.css` — a single refined light theme, fully self-contained.
- `app.js` — OS/arch detection and GitHub-Release-driven download buttons with a
  `releases/latest` fallback.
- `assets/` — logo, favicon, and `app.png` (swap the screenshot for a fresh
  capture whenever the app UI changes; capture at 2× for a crisp image).

## How downloads work

On load, `app.js` fetches
`https://api.github.com/repos/kingnazz/ArcScan/releases/latest` and maps the
release assets to download buttons:

| Button           | Asset pattern         |
| ---------------- | --------------------- |
| Windows (x64)    | `*x64-setup.exe`      |
| Windows (ARM64)  | `*arm64-setup.exe`    |
| macOS (universal)| `*universal.dmg`      |

The visitor's OS/arch is detected and promoted as the big primary button. If the
GitHub API is unreachable or rate-limited (60 requests/hr per IP,
unauthenticated), every control falls back to the stable
`https://github.com/kingnazz/ArcScan/releases/latest` page — no dead buttons, and
**no API token is ever embedded in the page**.

## Security notes

- No backend, no cookies, no analytics, no third-party scripts or CDNs.
- A strict `Content-Security-Policy` meta tag limits network access to
  `api.github.com` and blocks framing/inline script injection.
- Installers are linked **directly from GitHub Releases** — the site never
  re-hosts binaries, so it can't serve a tampered file.
- A "Verify your download (SHA-256)" link points at the release for integrity
  checking. (Publish a `checksums.txt` with each release to make this concrete.)

## Run locally

Just open `index.html`, or serve the folder:

```sh
cd site
python3 -m http.server 4000
# visit http://localhost:4000
```

## Deploy

Pushed to `main`, the `.github/workflows/pages.yml` workflow publishes this
folder to GitHub Pages at `https://kingnazz.github.io/ArcScan/`. Enable Pages
once under **Settings → Pages → Source: GitHub Actions**.
