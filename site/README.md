# ArcScan download site

A small, dependency-free static site that presents ArcScan and serves the newest
installer for each platform. It reads the latest GitHub Release at runtime, so it
never needs editing when a new version ships.

## Files

- `index.html` is the page (hero, features, preview, install steps, FAQ, CTA),
  including SEO metadata and `SoftwareApplication` structured data.
- `app.v2.css` is a single dark theme, fully self-contained.
- `app.v2.js` handles OS/arch detection and the GitHub-Release-driven download
  buttons, with a `releases/latest` fallback.
- `assets/` holds the logo, favicon, and `app-dark.png` (swap the screenshot for
  a fresh capture whenever the app UI changes; capture at 2x for a crisp image).

## How downloads work

On load, `app.v2.js` fetches
`https://api.github.com/repos/kingnazz/ArcScan/releases/latest` and maps the
release assets to download buttons:

| Button           | Asset pattern         |
| ---------------- | --------------------- |
| Windows (x64)    | `*x64-setup.exe`      |
| Windows (ARM64)  | `*arm64-setup.exe`    |
| macOS (universal)| `*universal.dmg`      |

The visitor's OS and architecture are detected and promoted as the primary
button. If the GitHub API is unreachable or rate-limited (60 requests per hour
per IP, unauthenticated), every control falls back to the stable
`https://github.com/kingnazz/ArcScan/releases/latest` page, so there are no dead
buttons, and no API token is ever embedded in the page.

## Security notes

- No backend, no cookies, no analytics, no third-party scripts or CDNs.
- A strict `Content-Security-Policy` meta tag limits network access to
  `api.github.com` and blocks framing and inline script injection.
- Installers are linked directly from GitHub Releases. The site never re-hosts
  binaries, so it cannot serve a tampered file.
- A "SHA-256 checksums" link points at the release for integrity checking.

## Filenames and caching

Asset filenames (`app.v2.css`, `app.v2.js`, `app-dark.png`) are versioned by name.
If the design changes in a way that could clash with a cached older stylesheet,
rename the file so a stale cache cannot pair an old stylesheet with new markup.

## Run locally

Open `index.html`, or serve the folder:

```sh
cd site
python3 -m http.server 4000
# visit http://localhost:4000
```

## Deploy

Pushed to `main`, the `.github/workflows/pages.yml` workflow publishes this
folder to GitHub Pages at `https://kingnazz.github.io/ArcScan/`. Enable Pages
once under Settings, Pages, Source: GitHub Actions.
