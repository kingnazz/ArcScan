# In-app auto-update — setup

ArcScan has a built-in updater: on launch it checks this repo's Releases feed,
and if a newer **signed** build exists it shows an "ArcScan vX is available →
Update now" banner that downloads, installs, and relaunches the app. The
header's cloud-download button re-checks on demand.

Because this repo is **public**, the updater reads its own GitHub Releases
directly — no separate repo, no personal access token. You just need a signing
key (updates must be signed) and two secrets.

## 1. Generate your signing keypair (required)

The updater verifies every download against a public key baked into the app
(`plugins.updater.pubkey` in `src-tauri/tauri.conf.json`); you sign releases
with the matching **private** key.

> ⚠️ The public key currently committed is a **build-time placeholder** — its
> private half is not available, so you **must** generate your own keypair and
> replace the placeholder before publishing a release.

```bash
npm run tauri signer generate -- -w arcscan.key
# prints your PUBLIC key and writes the PRIVATE key to ./arcscan.key
```

Put the printed **public key** into `src-tauri/tauri.conf.json`
(`plugins.updater.pubkey`), replacing the placeholder, and commit that change.

> Keep `arcscan.key` private — never commit it. Anyone with it can sign updates.

## 2. Add two secrets

On this repo, Settings → Secrets and variables → Actions:

| Secret | Value |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | contents of `arcscan.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | the password you set (`""` if none) |

That's it — the release workflow publishes to this repo's Releases with the
built-in `GITHUB_TOKEN`.

## 3. Publish a release

Bump the version in `package.json`, `src-tauri/Cargo.toml`, and
`src-tauri/tauri.conf.json` (all three must match), commit, then:

**Actions → "Publish Release" → Run workflow →** enter the tag (e.g. `v1.5.1`).

The workflow builds **signed** installers for Windows x64/ARM64 and macOS
universal, generates `latest.json`, and publishes them as a GitHub Release
(marked "latest"). Within a minute, every installed copy of ArcScan sees the
update on its next launch (or when the user clicks the cloud button) and can
one-click update.

## How it flows

```
Publish Release workflow
  ├─ build (signed) → *-setup.exe/.dmg/.app.tar.gz + .sig
  ├─ gen-latest-json.mjs → latest.json  (urls + signatures per platform)
  └─ publish → this repo's Releases (installers + latest.json, marked latest)

Installed ArcScan on launch
  ├─ reads endpoint -> releases/latest/download/latest.json
  ├─ newer version? verify signature against pubkey
  └─ banner → download → install → relaunch
```

## Notes

- **Version must go up** for the updater to offer an update (semver compare).
- The regular `build.yml` (PR/main) still produces **unsigned** installers for
  quick manual download — it needs no secrets. Only `release.yml` signs and
  feeds the updater.
- Windows updates install with the NSIS installer in `passive` mode (a brief
  progress dialog, no clicks). macOS swaps the app bundle and relaunches.
- Code signing (Apple/Windows certs) is separate from *update* signing; without
  it the **first** manual install still shows the OS "unidentified developer" /
  SmartScreen prompt once. Auto-updates after that are seamless.
