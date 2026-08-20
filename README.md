# Aloe Desktop

Aloe Desktop is the native desktop agent for Aloe. It connects a local machine to the Aloe web app so Aloe can safely work with granted folders, run local commands, manage terminal sessions, open local URLs, show notifications, and report recent activity back to the user.

The app is built with Tauri 2, Rust, React 19, Vite, and Bun.

## Features

- Register this device with a setup token from Aloe Integrations.
- Maintain a websocket connection to the Aloe backend.
- Grant and revoke local folder access, permanently for the device or temporarily for a single chat.
- Search, read, create, update, delete, and patch files inside granted folders.
- Run commands and terminal sessions with approval controls.
- Drive a real local Chrome over the DevTools Protocol so Aloe can use sites behind the user's own logins.
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

- `ask`: every command request is queued for explicit approval. A denial tells Aloe it was denied, so it stops rather than retrying the same command through another tool.
- `auto`: Aloe's own model judges each command against the conversation it came from (via the backend) before running it. This judgement is the only gate — there is no local blocklist in front of it, because the previous one rejected pipes, `&&`, and anything containing "curl" or "deploy", which is most real development work and left auto mode behaving exactly like `ask`. A command the check clears runs unattended. A command it declines does **not** reach the approval queue — it is refused on the spot and the judge's reason is returned to Aloe as the tool's error, so the model can explain what was blocked and propose something narrower in the same turn. Note the consequence: in `auto` there is no way to approve a command the judge rejected; switch to `ask` if you want the final say on every command.

  The two not-safe outcomes are kept distinct. A *verdict* refuses. A *failed check* — network error, timeout, unparseable answer — has judged nothing, so it falls back to the approval queue rather than refusing a command on no evidence.
- `all`: command approvals are disabled.

File operations are still limited to folders the user has explicitly granted.

## Folder Access

Two kinds of grant, with the same enforcement:

- **Device folders**, added in this app, reachable by every conversation and every background task for as long as they are listed.
- **Chat folders**, shared from the Aloe web app's chat composer. Pressing the folder button there asks this device to open its native folder dialog; the folder the user picks becomes reachable from that one conversation and nowhere else, until they remove it in the same place. They are listed read-only under **Folder access** so it is always visible on the device what a chat can reach.

The browser never names a path — it can only ask for the dialog, and is told the result. Aloe Desktop keeps its own copy of the chat grants and re-checks every job against them, so a job naming a conversation it was not granted still fails here.

Sensitive-path write rules (`.git/`, `node_modules/`, build output, `.env`, keys) apply identically to both kinds.

## Browser Automation

Aloe drives Chrome (or Chromium/Edge) through the DevTools Protocol on `127.0.0.1:9333`. If something is already listening there, Aloe attaches to it; otherwise it launches the browser itself.

Two profiles are available:

- `default` — the user's everyday Chrome profile, with their existing logins. Chrome must not already be running with that profile, because a second instance cannot open a debugging port on a locked profile. Aloe reports this explicitly if the port never comes up.
- `aloe` — a separate profile under `Aloe Desktop/browser-profile`, which can run alongside a normal Chrome window but starts signed out. Logins persist there between sessions.

Browser actions are **not** gated by the command-approval modes above: navigation, clicks, and typing run as soon as they are requested. Page content is untrusted input — a page that instructs the assistant to do something is data about that page, not a request from the user.

Aloe never launches the browser headless. The window is visible so the user can watch and take over.

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
