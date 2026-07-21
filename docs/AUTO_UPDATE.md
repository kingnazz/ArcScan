# In-app auto-update — setup

ArcScan has a built-in updater: on launch it checks a public feed, and if a
newer **signed** build exists it shows an "ArcScan vX is available → Update now"
banner that downloads, installs, and relaunches the app. The header's
cloud-download button re-checks on demand.

The app side is already wired up. To turn it on you need to do three one-time
things, because updates must be **signed** and served from a **public** feed
(this source repo is private, so its release assets can't be downloaded
anonymously).

## 1. Create a public releases repo

Create an **empty public** repo to host the installers + `latest.json`, e.g.
`kingnazz/arcscan-releases`. Nothing but release assets live here; your source
stays private.

> If you pick a different name, update `RELEASES_REPO` in
> `.github/workflows/release.yml` **and** the `endpoints` URL in
> `src-tauri/tauri.conf.json` to match.

## 2. Generate your signing keypair (required)

The updater verifies every download against a public key baked into the app
(`plugins.updater.pubkey` in `src-tauri/tauri.conf.json`) and you sign releases
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
Then add these **repository secrets** (Settings → Secrets and variables →
Actions) on the **private** source repo:

| Secret | Value |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | contents of `arcscan.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | the password you set (`""` if none) |
| `RELEASES_TOKEN` | a Personal Access Token (classic: `repo` scope, or fine-grained with **Contents: read/write** on the releases repo) that can create releases in the public releases repo |

> Keep `arcscan.key` private — never commit it. Anyone with it can sign updates.

## 3. Publish a release

Bump the version in `package.json`, `src-tauri/Cargo.toml`, and
`src-tauri/tauri.conf.json` (all three must match), commit, then:

**Actions → "Publish Release" → Run workflow →** enter the tag (e.g. `v1.5.1`).

The workflow builds **signed** installers for Windows x64/ARM64 and macOS
universal, generates `latest.json`, and publishes everything to the public
releases repo. Within a minute, every installed copy of ArcScan will see the
update on its next launch (or when the user clicks the cloud button) and can
one-click update.

## How it flows

```
Publish Release workflow
  ├─ build (signed) → *-setup.exe/.dmg/.app.tar.gz + .sig
  ├─ gen-latest-json.mjs → latest.json  (urls + signatures per platform)
  └─ publish → PUBLIC releases repo (installers + latest.json)

Installed ArcScan on launch
  ├─ reads endpoints -> latest.json
  ├─ newer version? verify signature against pubkey
  └─ banner → download → install → relaunch
```

## Notes

- **Version must go up** for the updater to offer an update (semver compare).
- The regular `build.yml` (PR/main) still produces **unsigned** installers for
  quick manual download — it doesn't need any of the secrets above. Only
  `release.yml` signs and feeds the updater.
- Windows updates install with the NSIS installer in `passive` mode (a brief
  progress dialog, no clicks). macOS swaps the app bundle and relaunches.
- Unsigned first install still shows the OS "unidentified developer"/SmartScreen
  prompt once; auto-updates after that are seamless.
