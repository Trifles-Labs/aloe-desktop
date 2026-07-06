# Aloe Desktop

Aloe Desktop is the native desktop agent for Aloe. It connects a local machine to the Aloe web app so Aloe can safely work with granted folders, run local commands, manage terminal sessions, open local URLs, show notifications, and report recent activity back to the user.

The app is built with Tauri 2, Rust, React 19, Vite, and Bun.

## Features

- Register this device with a setup token from Aloe Integrations.
- Maintain a websocket connection to the Aloe backend.
- Grant and revoke local folder access.
- Search, read, create, update, delete, and patch files inside granted folders.
- Run commands and terminal sessions with approval controls.
- Choose command trust mode: ask every time, trusted coding commands, or allow all.
- Show pending approvals and recent agent activity in the desktop UI.
- Support tray behavior, startup preferences, notifications, and silent auto-updates.

## Project Structure

```text
.
|-- src/                  # React desktop UI
|-- src-tauri/            # Tauri/Rust application shell and local agent runtime
|-- build_release.py      # Interactive release build helper
|-- UPDATER.md            # Auto-update release process
|-- package.json          # Bun scripts and frontend dependencies
`-- vite.config.ts        # Vite configuration
```

## Requirements

- Bun
- Rust stable and Cargo
- Tauri 2 system dependencies for your OS
- A valid Aloe setup token to connect the app

For Linux, install the native packages required by Tauri/WebKitGTK for your distribution before running or building the app.

## Development

Install dependencies:

```sh
bun install
```

Run the desktop app in development mode:

```sh
bun run tauri:dev
```

The Tauri dev server starts Vite on `127.0.0.1:1420` through the `beforeDevCommand` configured in `src-tauri/tauri.conf.json`.

Run a frontend typecheck and production Vite build:

```sh
bun run build
```

Build the native app:

```sh
bun run tauri:build
```

## Connecting to Aloe

1. Open Aloe Desktop.
2. Copy a setup token from the Aloe Integrations page.
3. Paste the token into the login screen.
4. Grant folders that Aloe is allowed to inspect or modify.
5. Review command approvals from the Desktop controls page when commands are requested.

In debug builds, the app defaults to `http://127.0.0.1:8080` for the Aloe backend. In release builds, it defaults to `https://api.247autoarmy.in/`.

To compile against a different backend URL, set `ALOE_BACKEND_URL` at build time:

```sh
ALOE_BACKEND_URL=https://your-api.example.com bun run tauri:build
```

## Local Data

Aloe Desktop stores its local configuration in the operating system config directory under `Aloe Desktop/config.json`. The config includes the registered agent id, credential, user profile, granted folders, desktop preferences, recent actions, and terminal session metadata.

Use **Log out** in the app to reset the agent connection and remove stored credentials.

## Command Approval Modes

- `ask`: every command request is queued for explicit approval.
- `trusted_coding`: common project verification commands are allowed, while destructive or compound commands still require approval.
- `all`: command approvals are disabled.

File operations are still limited to folders the user has explicitly granted.

## Releases and Updates

Auto-update behavior is configured in `src-tauri/tauri.conf.json`. Release builds create updater artifacts, and the app checks:

```text
https://github.com/Trifles-Labs/aloe-desktop/releases/latest/download/latest.json
```

See [UPDATER.md](UPDATER.md) for signing key setup, versioning, tagging, publishing, and local update testing.

You can also use the release helper:

```sh
python build_release.py 1.2.3
```

On Linux, choose bundles with:

```sh
python build_release.py 1.2.3 --linux-bundles deb,rpm
```

## Troubleshooting

If registration fails, verify the setup token, backend URL, and that the backend is reachable from the desktop machine.

If the socket stays disconnected, log out and register again with a fresh setup token. The app also reconnects automatically with backoff when the backend is temporarily unavailable.

If folder operations fail, remove and re-add the folder so the stored path is refreshed and canonicalized.

If Linux rendering fails under Wayland, the app enables software rendering for Wayland sessions at startup.

## Useful Commands

```sh
bun install              # install JavaScript dependencies
bun run tauri:dev        # run the desktop app locally
bun run build            # typecheck and build the web UI
bun run tauri:build      # build native installers/artifacts
python build_release.py  # guided release build
```
