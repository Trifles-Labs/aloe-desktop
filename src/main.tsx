import React, { useCallback, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { Flower2, Leaf, MonitorCheck } from "lucide-react";

import { useToasts, ToastContainer } from "./toast";
import { ButterflyDecor } from "./components/ButterflyDecor";
import { AuthScreen } from "./components/AuthScreen";
import { ConnectionPanel } from "./components/ConnectionPanel";
import { FoldersPanel } from "./components/FoldersPanel";
import { ApprovalsPanel } from "./components/ApprovalsPanel";
import { SearchPanel } from "./components/SearchPanel";
import { ActivityList } from "./components/ActivityList";
import { DEFAULT_CONFIG } from "./types";
import type { AgentConfig, PendingApproval } from "./types";
import "./styles.css";

function App() {
  const [config, setConfig] = useState<AgentConfig>(DEFAULT_CONFIG);
  const [pending, setPending] = useState<PendingApproval[]>([]);
  const [setupToken, setSetupToken] = useState("");
  const { toasts, toast, dismiss } = useToasts();

  const authenticated = Boolean(config.agentId && config.credential);
  const connected = config.socketStatus === "connected";

  // ── Data refresh ────────────────────────────────────────────────────────────

  const refresh = useCallback(async () => {
    const [nextConfig, nextPending] = await Promise.all([
      invoke<AgentConfig>("get_config"),
      invoke<PendingApproval[]>("get_pending_approvals"),
    ]);
    setConfig(nextConfig);
    setPending(nextPending);
  }, []);

  useEffect(() => {
    void refresh();
    const id = window.setInterval(() => void refresh(), 1500);
    return () => window.clearInterval(id);
  }, [refresh]);

  // ── Handlers ────────────────────────────────────────────────────────────────

  const connect = async () => {
    try {
      const next = await invoke<AgentConfig>("register_agent", { token: setupToken });
      setSetupToken("");
      setConfig(next);
      toast("Aloe Desktop registered — opening socket connection.", "success");
    } catch (err) {
      toast(`Connection failed: ${err instanceof Error ? err.message : String(err)}`, "error");
    }
  };

  const resetConnection = async () => {
    try {
      const next = await invoke<AgentConfig>("reset_agent_connection");
      setConfig(next);
      toast("Logged out. Paste a fresh setup token to reconnect.", "info");
    } catch (err) {
      toast(`Reset failed: ${err instanceof Error ? err.message : String(err)}`, "error");
    }
  };

  const addFolder = async () => {
    try {
      const next = await invoke<AgentConfig>("add_folder");
      const added = next.folders[next.folders.length - 1];
      setConfig(next);
      if (added) toast(`Folder granted: ${added.label ?? added.path}`, "success");
    } catch (err) {
      toast(`Could not add folder: ${err instanceof Error ? err.message : String(err)}`, "error");
    }
  };

  const removeFolder = async (path: string) => {
    try {
      const next = await invoke<AgentConfig>("remove_folder", { path });
      setConfig(next);
      toast("Folder removed.", "info");
    } catch (err) {
      toast(`Could not remove folder: ${err instanceof Error ? err.message : String(err)}`, "error");
    }
  };

  const approve = async (jobId: string, approved: boolean) => {
    try {
      await invoke("approve_command", { jobId, approved });
      await refresh();
      toast(approved ? "Command approved and running." : "Command denied.", approved ? "success" : "info");
    } catch (err) {
      toast(`Approval failed: ${err instanceof Error ? err.message : String(err)}`, "error");
    }
  };

  const setAlwaysAllow = async (enabled: boolean) => {
    try {
      const next = await invoke<AgentConfig>("set_always_allow_commands", { enabled });
      setConfig(next);
      toast(enabled ? "Always-allow enabled." : "Per-command approval required again.", "info");
    } catch (err) {
      toast(`Setting failed: ${err instanceof Error ? err.message : String(err)}`, "error");
    }
  };

  // ── Render ──────────────────────────────────────────────────────────────────

  return (
    <main className="app-shell">
      {/* Botanical background decorations */}
      <div style={{ position: "fixed", right: 24, top: 80, color: "#6f8747", opacity: 0.06, pointerEvents: "none", zIndex: 0 }}>
        <Leaf size={120} style={{ transform: "rotate(20deg)" }} />
      </div>
      <div style={{ position: "fixed", left: 16, bottom: 60, color: "#d98f82", opacity: 0.08, pointerEvents: "none", zIndex: 0 }}>
        <ButterflyDecor style={{ width: 80, height: 56 }} />
      </div>
      <div style={{ position: "fixed", right: 60, bottom: 40, color: "#6f8747", opacity: 0.06, pointerEvents: "none", zIndex: 0 }}>
        <Flower2 size={72} style={{ transform: "rotate(15deg)" }} />
      </div>

      {/* Topbar */}
      <section className="topbar">
        <div className="brand-lockup">
          <div className="brand-mark"><Leaf size={18} /></div>
          <div className="topbar-copy">
            <p className="eyebrow">Aloe Desktop</p>
            <h1>Local agent</h1>
          </div>
        </div>
        {authenticated && (
          <div className={connected ? "status online" : "status offline"}>
            <span className="status-dot" />
            <MonitorCheck size={14} />
            {connected ? "Socket connected" : config.socketStatus || "Not connected"}
          </div>
        )}
      </section>

      <div style={{ position: "relative", zIndex: 1 }}>
        {!authenticated ? (
          <AuthScreen
            setupToken={setupToken}
            onTokenChange={setSetupToken}
            onConnect={() => void connect()}
          />
        ) : (
          <div className="content-stack">
            {!connected && (
              <div className="connection-warning">
                Authenticated but socket is {config.socketStatus || "disconnected"}.
                {config.socketError ? ` Error: ${config.socketError}` : ""}
              </div>
            )}

            <div className="grid">
              <ConnectionPanel config={config} onReset={() => void resetConnection()} />
              <FoldersPanel
                folders={config.folders}
                onAdd={() => void addFolder()}
                onRemove={(path) => void removeFolder(path)}
              />
            </div>

            <ApprovalsPanel
              config={config}
              pending={pending}
              onRefresh={() => void refresh()}
              onApprove={(jobId, approved) => void approve(jobId, approved)}
              onToggleAlwaysAllow={(enabled) => void setAlwaysAllow(enabled)}
            />

            <SearchPanel folders={config.folders} onToast={toast} />

            <ActivityList actions={config.recentActions} />
          </div>
        )}
      </div>

      <ToastContainer toasts={toasts} onDismiss={dismiss} />
    </main>
  );
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
