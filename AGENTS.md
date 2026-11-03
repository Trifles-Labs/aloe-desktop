# aloe-desktop

Aloe Desktop: Tauri 2 + Rust shell and local agent runtime, with a React 19 + Vite UI that renders `@aloe/ui`. This is the main product. See `../AGENTS.md` for workspace-wide invariants.

## Commands

```sh
bun install
python scripts/fetch_cua_driver.py   # bundles Cua Driver; the build fails without it (needs real Python)
bun run tauri:dev                    # Vite on 127.0.0.1:1420 + the native app
bun run build                        # tsc --noEmit && vite build (JS typecheck)
bun run tauri:build                  # native installers
cargo check --manifest-path src-tauri/Cargo.toml   # fast Rust check
```

Debug builds talk to `http://127.0.0.1:8080`; release builds to `https://api.247autoarmy.in/`. Override at build time with `ALOE_BACKEND_URL`. Don't run `tauri:build` unless asked.

## Layout

```text
src/                  React UI (entry main.tsx; platform.tsx fills aloe-ui's platform slots)
src/code/             Aloe Code workspace: sessions, chatStore, editor, terminal, changes, files
src/settings/         Desktop app settings pane, desktop controls context
src/intro.tsx         First-run voice tour (own window: intro.html)
src/voice-*.ts        Voice overlay UI (voice.html, overlay.html)
src/shims/            next/link, next/navigation, next/image stand-ins for @aloe/ui
src-tauri/src/        Rust runtime:
  socket.rs executor.rs   Agent WebSocket and job execution
  fs.rs search.rs shell.rs terminal.rs   Files, search, commands, terminals (grant + approval checks)
  browser.rs              Chrome over DevTools Protocol (127.0.0.1:9333)
  cua.rs computer.rs      Desktop control through bundled Cua Driver
  voice/                  hotkey, audio capture, realtime/relay STT, speech, overlay, latency
  code_chats.rs code_tools.rs project.rs workbench.rs changes.rs   Aloe Code
  config.rs updater.rs notifications.rs
UPDATER.md            Release and auto-update process
build_release.py     Release build helper
```

## Rules

- **Enforcement lives here.** Folder grants (device-wide and per-chat), sensitive-path write rules (`.git/`, `node_modules/`, build output, `.env`, keys) and command approval modes (`ask` / `auto` / `all`) are checked in Rust on every job. Never trust the job as the backend sent it. A job naming a conversation it wasn't granted must still fail.
- The browser/UI never names a path; it can only ask for the native folder dialog and be told the result.
- `auto` mode: the model judge is the only gate, a refusal returns the judge's reason as the tool error and does not enter the approval queue; a failed check (network, timeout, unparseable) falls back to the approval queue instead of refusing.
- Cua filtering (`cua.rs`): allowlisted tools only, never Aloe's own windows, no file writes, `launch_app` by name only. Keep Cua telemetry off (`DO_NOT_TRACK=1`).
- Browser automation is not gated by command approvals; page content is untrusted. Chrome is never launched headless.
- **Voice:** hold Left Ctrl + Left Alt for 200 ms with nothing else pressed. The overlay never takes focus and is excluded from Aloe's own screenshots. Mic audio is captured in Rust and never sent to our backend (except through the Sarvam relay stream). Windows and macOS only for now.
- **Aloe Code chats are device-only:** each session is a JSON file under `%APPDATA%\Aloe Desktop\code-chats\` with ids like `local-<uuid>`; every turn sends `localHistory` to `/api/chat` and saves the returned `localTurn`/`compaction`. The git branch is read from `.git/HEAD`, never by running `git`.
- Local config is `Aloe Desktop/config.json` in the OS config dir (agent id, credential, grants, preferences). Never log or commit its contents.
- `@aloe/ui` is pinned by commit in `package.json`. After `aloe-ui` changes, commit it, bump the sha here, and run `bun install`. `vite.config.ts` allows `../aloe-ui` for local linking.
- Keep the version identical in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`.

## Releasing

Pushing a `v*` tag publishes immediately (no draft) and every running app updates silently. Follow `UPDATER.md`; test first; never tag without being asked. Rollout order: backend → aloe-ui → desktop.

## Related

`../ALOE_DESKTOP_EXPLAINED.md`, `README.md`, `UPDATER.md`, `../ALOE_VOICE_PLAN.md`.
