/* Startup preferences for Aloe Desktop. A settings pane the desktop adds to the
   shared Settings page; the web app has no equivalent. */

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { Info, MonitorDown, Power } from "lucide-react";

import Switch from "@aloe/ui/components/ui/Switch";
import { PaneHeader } from "@aloe/ui/components/settings/SettingsPane";
import type { AgentConfig } from "../types";

type DesktopPreferences = { runOnStartup: boolean; startMinimized: boolean };

const preferencesOf = (config: AgentConfig): DesktopPreferences => ({ runOnStartup: config.runOnStartup, startMinimized: config.startMinimized });

export default function DesktopAppPane() {
  const [preferences, setPreferences] = useState<DesktopPreferences | null>(null);
  const [version, setVersion] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    invoke<AgentConfig>("get_config")
      .then((config) => setPreferences(preferencesOf(config)))
      .catch((cause: unknown) => setError(cause instanceof Error ? cause.message : "Could not load desktop preferences."));
    getVersion().then(setVersion).catch(() => setVersion(null));
  }, []);

  const update = async (kind: "startup" | "minimized", enabled: boolean) => {
    setBusy(true);
    try {
      const config = await invoke<AgentConfig>(kind === "startup" ? "set_run_on_startup" : "set_start_minimized", { enabled });
      setPreferences(preferencesOf(config));
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not update desktop preferences.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <PaneHeader title="Desktop app" blurb="How Aloe Desktop behaves when you sign in to your computer." />

      {preferences ? (
        <div className="settings-group divide-y divide-edge overflow-hidden px-5">
          <div className="flex items-center justify-between gap-4 py-4">
            <div className="flex min-w-0 items-start gap-3">
              <Power className="mt-0.5 h-4 w-4 shrink-0 text-moss" />
              <div className="min-w-0">
                <p className="text-[13px] font-medium text-ink">Run Aloe Desktop on startup</p>
                <p className="mt-0.5 text-xs leading-5 text-ink-soft">Launch Aloe when you sign in to your computer.</p>
              </div>
            </div>
            <Switch checked={preferences.runOnStartup} disabled={busy} label="Run Aloe Desktop on startup" onChange={(enabled) => void update("startup", enabled)} />
          </div>
          <div className="flex items-center justify-between gap-4 py-4">
            <div className="flex min-w-0 items-start gap-3">
              <MonitorDown className="mt-0.5 h-4 w-4 shrink-0 text-moss" />
              <div className="min-w-0">
                <p className="text-[13px] font-medium text-ink">Start minimized</p>
                <p className="mt-0.5 text-xs leading-5 text-ink-soft">When launched at sign-in, stay in the system tray until opened.</p>
              </div>
            </div>
            <Switch checked={preferences.startMinimized} disabled={busy || !preferences.runOnStartup} label="Start minimized" onChange={(enabled) => void update("minimized", enabled)} />
          </div>
          <div className="flex items-center justify-between gap-4 py-4">
            <div className="flex min-w-0 items-start gap-3">
              <Info className="mt-0.5 h-4 w-4 shrink-0 text-moss" />
              <div className="min-w-0">
                <p className="text-[13px] font-medium text-ink">Aloe Desktop</p>
                <p className="mt-0.5 text-xs leading-5 text-ink-soft">Version {version ?? "…"}</p>
              </div>
            </div>
          </div>
        </div>
      ) : null}

      {error ? <p className="mt-3 text-xs text-danger">{error}</p> : null}
    </>
  );
}
