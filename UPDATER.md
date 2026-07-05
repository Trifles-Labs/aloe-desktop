# Aloe Desktop — Auto-Update Release Guide

## How it works

The desktop app checks `https://github.com/Trifles-Labs/aloe-desktop/releases/latest/download/latest.json` on launch (see `plugins.updater.endpoints` in `src-tauri/tauri.conf.json`). That URL always resolves to the `latest.json` asset on the most recent **published, non-draft, non-prerelease** GitHub release.

`.github/workflows/release.yml` builds and signs installers for macOS, Windows, and Linux via `tauri-apps/tauri-action`, which also generates `latest.json` and uploads it as a release asset. Releases are created as **drafts** (`releaseDraft: true`), so nothing ships to users until you manually publish the draft on GitHub — that's your rollout gate.

If the update is available, the app downloads and installs it silently, then shows a "Restart now" banner.

---

## One-time setup: signing keys

Run this once and keep the output safe:

```sh
bunx @tauri-apps/cli signer generate -w ~/.tauri/aloe.key
```

Two things are printed:
- **Public key** — already set in `src-tauri/tauri.conf.json` under `plugins.updater.pubkey`
- **Private key file** — stored at `~/.tauri/aloe.key`

The private key must be set as the `TAURI_SIGNING_PRIVATE_KEY` (and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if used) repo secrets — the release workflow already reads them from GitHub Actions secrets. Never commit the private key.

---

## Cutting a release

### 1. Bump the version

Edit `src-tauri/tauri.conf.json`:
```json
"version": "1.2.0"
```

### 2. Tag and push

```sh
git tag v1.2.0
git push origin v1.2.0
```

Pushing a `v*` tag triggers `.github/workflows/release.yml`, which builds installers for all platforms, signs them, and creates a **draft** GitHub release named `Aloe Desktop v1.2.0` with all installer artifacts and `latest.json` attached.

You can also trigger the workflow manually via `workflow_dispatch` from the Actions tab.

### 3. Publish the draft

Once the workflow finishes, open the draft release on GitHub, review the artifacts/notes, and click **Publish release**. As soon as it's published, `https://github.com/Trifles-Labs/aloe-desktop/releases/latest/download/latest.json` starts pointing at it, and running apps will pick it up on their next update check.

---

## Version format

Use plain semver: `1.0.0`, `1.2.3`, etc., matching the pushed tag (`v1.2.3`).

---

## Testing an update locally

1. Build and install the app at an older version (e.g. `0.9.0`).
2. Tag and push a newer version (e.g. `v1.0.0`), let CI build it, then publish the draft release.
3. Run the `0.9.0` build — it should download the update silently and show "Restart now".
4. After restart, the app should be at `1.0.0`.
