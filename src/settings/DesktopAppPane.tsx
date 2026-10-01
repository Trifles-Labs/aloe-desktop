/* Everything Aloe Desktop controls on this computer, as one pane the desktop
   adds to the shared Settings page; the web app has no equivalent. It used to
   be split between this pane (startup preferences) and a separate Desktop
   controls page — now it is one place: how the app starts, what the local
   agent can reach, what it may run without asking, and what it has been doing. */

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { Info, MonitorCheck, MonitorDown, PlugZap, Power } from "lucide-react";

import Switch from "@aloe/ui/components/ui/Switch";
import { PaneHeader } from "@aloe/ui/components/settings/SettingsPane";
import { ActivityList } from "../components/ActivityList";
import { ApprovalsPanel } from "../components/ApprovalsPanel";
import { ConnectionPanel } from "../components/ConnectionPanel";
import { ControlGroup, ControlRow } from "../components/ControlGroup";
import { DesktopControlPanel } from "../components/DesktopControlPanel";
import { FoldersPanel } from "../components/FoldersPanel";
import type { AgentConfig } from "../types";
import { useDesktopControls } from "./DesktopControlsContext";

export default function DesktopAppPane() {
  const { config, pending, setConfig, onRefresh, onReset, onAddFolder, onRemoveFolder, onSetCommandTrustMode, onSetDesktopControl } =
    useDesktopControls();
  const [version, setVersion] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const connected = config.socketStatus === "connected";

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion(null));
  }, []);

  const update = async (kind: "startup" | "minimized", enabled: boolean) => {
    setBusy(true);
    try {
      setConfig(await invoke<AgentConfig>(kind === "startup" ? "set_run_on_startup" : "set_start_minimized", { enabled }));
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not update desktop preferences.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <PaneHeader
        title="Desktop app"
        blurb={`The local agent on ${config.deviceName}: how it starts, what it can reach, what it may run without asking, and what it has been doing.`}
        actions={
          <span
            className={`inline-flex items-center gap-2 rounded-full px-3 py-1.5 text-xs font-semibold ${
              connected ? "bg-sage text-ink" : "border border-edge bg-surface text-ink-soft"
            }`}
          >
            <span className={`h-1.5 w-1.5 rounded-full ${connected ? "bg-moss watch-pulse" : "bg-ink-soft/40"}`} />
            <MonitorCheck className="h-3.5 w-3.5" />
            {connected ? "Connected" : config.socketStatus || "Disconnected"}
          </span>
        }
      />

      {!connected ? (
        <div className="mb-6 flex items-start gap-3 rounded-xl border border-clay/40 bg-clay/8 px-4 py-3 text-[13px] leading-5 text-danger">
          <PlugZap className="mt-0.5 h-4 w-4 shrink-0" />
          <span>
            The local agent is {config.socketStatus || "disconnected"}. Aloe can't reach this computer until it reconnects.
            {config.socketError ? ` ${config.socketError}` : ""}
          </span>
        </div>
      ) : null}

      <ControlGroup label="Startup" footnote={error ? <span className="text-danger">{error}</span> : undefined}>
        <ControlRow
          icon={Power}
          title="Run Aloe Desktop on startup"
          detail="Launch Aloe when you sign in to your computer."
          control={<Switch checked={config.runOnStartup} disabled={busy} label="Run Aloe Desktop on startup" onChange={(enabled) => void update("startup", enabled)} />}
        />
        <ControlRow
          icon={MonitorDown}
          title="Start minimized"
          detail="When launched at sign-in, stay in the system tray until opened."
          control={
            <Switch
              checked={config.startMinimized}
              disabled={busy || !config.runOnStartup}
              label="Start minimized"
              onChange={(enabled) => void update("minimized", enabled)}
            />
          }
        />
        <ControlRow icon={Info} title="Aloe Desktop" detail={`Version ${version ?? "…"}`} />
      </ControlGroup>

      <ConnectionPanel config={config} onReset={onReset} />
      <FoldersPanel folders={config.folders} conversationFolders={config.conversationFolders} onAdd={onAddFolder} onRemove={onRemoveFolder} />
      <ApprovalsPanel config={config} pending={pending} onRefresh={onRefresh} onSetCommandTrustMode={onSetCommandTrustMode} />
      <DesktopControlPanel enabled={config.desktopControlEnabled} onSetEnabled={onSetDesktopControl} />
      <ActivityList actions={config.recentActions} />
    </>
  );
}
