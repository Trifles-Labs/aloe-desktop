# Aloe Desktop — Auto-Update Release Guide

## How it works

The desktop app checks `https://api.247autoarmy.in/api/updates/{target}/{arch}/{version}` on launch. If the backend returns a newer version, the app downloads it silently and shows a "Restart now" banner. The backend reads from the `AppRelease` collection in MongoDB. You manage releases through the analytics dashboard at `/updates`.

---

## One-time setup: signing keys

Run this once and keep the output safe:

```sh
bunx @tauri-apps/cli signer generate -w ~/.tauri/aloe.key
```

Two things are printed:
- **Public key** — already set in `src-tauri/tauri.conf.json` under `plugins.updater.pubkey`
- **Private key file** — stored at `~/.tauri/aloe.key`

The private key must be set as `TAURI_SIGNING_PRIVATE_KEY` when building. Never commit it.

---

## Building a release

### 1. Set the version

Edit `src-tauri/tauri.conf.json`:
```json
"version": "1.2.0"
```

### 2. Set the signing key env var

```sh
# Paste the private key content directly (the full "untrusted comment: ..." block)
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/aloe.key)"

# If you set a password when generating the key:
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="your-password"
```

On Windows (PowerShell):
```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content ~/.tauri/aloe.key -Raw
```

### 3. Build

```sh
bun run tauri:build
```

This produces installer bundles **and** their `.sig` signature files in `src-tauri/target/release/bundle/`.

---

## Output artifacts

After a successful build, find the updatable artifacts here:

| Platform | Artifact path (relative to `src-tauri/target/release/bundle/`) |
|---|---|
| Windows (NSIS) | `nsis/Aloe Desktop_1.2.0_x64-setup.exe` + `.exe.sig` |
| Windows (MSI) | `msi/Aloe Desktop_1.2.0_x64_en-US.msi` + `.msi.sig` |
| macOS (Apple Silicon) | `macos/Aloe Desktop.app.tar.gz` + `.sig` |
| macOS (Intel) | `macos/Aloe Desktop.app.tar.gz` + `.sig` *(cross-compiled)* |
| Linux | `appimage/aloe-desktop_1.2.0_amd64.AppImage.tar.gz` + `.sig` |

The `.sig` file contains the signature string you paste into the dashboard.

---

## Publishing a release

### 1. Upload the artifact files

Host the `.zip` / `.tar.gz` files somewhere with a stable public URL — your backend's static file server, S3, Cloudflare R2, etc.

If serving from the backend, drop them in a `public/` or `static/` directory and expose via a static route. The URL just needs to be reachable by the end-user's machine at update time.

### 2. Get the signature string

```sh
cat "src-tauri/target/release/bundle/nsis/Aloe Desktop_1.2.0_x64-setup.exe.sig"
```

Copy the full output including the `untrusted comment: ...` header line.

### 3. Create the release in the analytics dashboard

1. Open the analytics dashboard → **Updates** page
2. Click **New release**
3. Fill in:
   - **Version**: `1.2.0` (must match `tauri.conf.json`)
   - **Release date**: today
   - **Notes**: what changed
   - For each platform row, paste the download URL and the signature from the `.sig` file
4. Click **Save**

### 4. Promote to latest

In the releases table, click **Promote** on the new release. This sets `isLatest: true` and the backend will start serving it to desktop clients.

---

## Version format

Use plain semver: `1.0.0`, `1.2.3`, etc. The backend compares versions numerically — `1.2.0` is newer than `1.1.9`. Prefix `v` is stripped automatically.

---

## Testing an update locally

1. Build the app at version `0.9.0`
2. In the dashboard, create a release at version `1.0.0` with a real signed artifact URL, promote it to latest
3. Run the `0.9.0` build — it should download silently and show "Restart now"
4. After restart, the app should be at `1.0.0`

To test without a hosted artifact, you can temporarily point the download URL to a local file server (`npx serve` or Python's `http.server`) — the updater will follow any URL.
