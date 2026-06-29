import React, { useCallback, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { Flower2, Leaf, MonitorCheck } from "lucide-react";

import { useToasts, ToastContainer } from "./toast";
import { useAutoUpdate } from "./hooks/useAutoUpdate";
import { ButterflyDecor } from "./components/ButterflyDecor";
import { AuthScreen } from "./components/AuthScreen";
import { ConnectionPanel } from "./components/ConnectionPanel";
import { FoldersPanel } from "./components/FoldersPanel";
import { ApprovalsPanel } from "./components/ApprovalsPanel";
import { ActivityList } from "./components/ActivityList";
import { DesktopTitleBar } from "./components/DesktopTitleBar";
import { ThemeProvider } from "next-themes";
import Providers from "@/app/providers";
import AppLayout from "@/app/(app)/layout";
import ChatPage from "@/app/(app)/app/chat/page";
import ConversationsPage from "@/app/(app)/app/conversations/page";
import IntegrationsPage from "@/app/(app)/app/integrations/page";
import MobileLoginPage from "@/app/(app)/app/mobile-login/page";
import PlansPage from "@/app/(app)/app/plans/page";
import SettingsPage from "@/app/(app)/app/settings/page";
import UsagePage from "@/app/(app)/app/usage/page";
import BoardPage from "@/app/(app)/app/board/page";
import { usePathname } from "next/navigation";
import { DEFAULT_CONFIG } from "./types";
import type { AgentConfig, CommandTrustMode, PendingApproval } from "./types";
import "./web.css";

type DesktopPreferences = { runOnStartup: boolean; startMinimized: boolean };
const preferenceShape = (config: AgentConfig): DesktopPreferences => ({ runOnStartup: config.runOnStartup, startMinimized: config.startMinimized });
(window as Window & { __ALOE_DESKTOP__?: unknown }).__ALOE_DESKTOP__ = {
  getPreferences: async () => preferenceShape(await invoke<AgentConfig>("get_config")),
  setRunOnStartup: async (enabled: boolean) => preferenceShape(await invoke<AgentConfig>("set_run_on_startup", { enabled })),
  setStartMinimized: async (enabled: boolean) => preferenceShape(await invoke<AgentConfig>("set_start_minimized", { enabled })),
  openExternal: (url: string) => invoke<void>("open_external_url", { url }),
};

function DesktopRouter({ desktopPage }: { desktopPage: React.ReactNode }) {
  const pathname = usePathname();
  const pages: Record<string, React.ReactNode> = {
    "/app/chat": <ChatPage />,
    "/app/conversations": <ConversationsPage />,
    "/app/integrations": <IntegrationsPage />,
    "/app/mobile-login": <MobileLoginPage />,
    "/app/plans": <PlansPage />,
    "/app/settings": <SettingsPage />,
    "/app/usage": <UsagePage />,
    "/app/board": <BoardPage />,
    "/app/desktop": desktopPage,
  };

  const page = pages[pathname] ?? pages["/app/chat"];
  return <AppLayout>{page}</AppLayout>;
}

function App() {
  const [config, setConfig] = useState<AgentConfig>(DEFAULT_CONFIG);
  const [pending, setPending] = useState<PendingApproval[]>([]);
  const [setupToken, setSetupToken] = useState("");
  const { toasts, toast, dismiss } = useToasts();
  const { updateReady, restart } = useAutoUpdate();

  const authenticated = Boolean(config.agentId && config.credential && config.userToken);
  const connected = config.socketStatus === "connected";

  const persistUserToken = (nextConfig: AgentConfig) => {
    if (nextConfig.userToken) {
      window.localStorage.setItem("aloe_token", nextConfig.userToken);
    } else {
      window.localStorage.removeItem("aloe_token");
    }
    if (nextConfig.userProfile) {
      window.localStorage.setItem("aloe_desktop_user", JSON.stringify(nextConfig.userProfile));
    } else {
      window.localStorage.removeItem("aloe_desktop_user");
    }
  };

  // ── Data refresh ────────────────────────────────────────────────────────────

  const refresh = useCallback(async () => {
    const [nextConfig, nextPending] = await Promise.all([
      invoke<AgentConfig>("get_config"),
      invoke<PendingApproval[]>("get_pending_approvals"),
    ]);
    persistUserToken(nextConfig);
    setConfig(nextConfig);
    setPending(nextPending);
  }, []);

  useEffect(() => {
    void refresh();
    const id = window.setInterval(() => void refresh(), 1500);
    return () => window.clearInterval(id);
  }, [refresh]);

  useEffect(() => {
    if (config.userToken) {
      window.localStorage.setItem("aloe_token", config.userToken);
    } else {
      window.localStorage.removeItem("aloe_token");
    }
  }, [config.userToken]);

  useEffect(() => {
    const signOut = () => {
      void invoke<AgentConfig>("reset_agent_connection").then(setConfig).catch((error) => {
        toast(`Logout failed: ${error instanceof Error ? error.message : String(error)}`, "error");
      });
    };
    window.addEventListener("aloe:desktop-signout", signOut);
    return () => window.removeEventListener("aloe:desktop-signout", signOut);
  }, [toast]);

  useEffect(() => {
    window.localStorage.setItem("aloe_desktop_pending_approvals", String(pending.length));
    window.dispatchEvent(new CustomEvent("aloe:desktop-approvals", { detail: pending.length }));
  }, [pending.length]);

  // ── Handlers ────────────────────────────────────────────────────────────────

  const connect = async () => {
    try {
      const next = await invoke<AgentConfig>("register_agent", { token: setupToken });
      persistUserToken(next);
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

  const setCommandTrustMode = async (mode: CommandTrustMode) => {
    try {
      const next = await invoke<AgentConfig>("set_command_trust_mode", { mode });
      setConfig(next);
      const message = mode === "all"
        ? "All command approvals disabled."
        : mode === "trusted_coding"
          ? "Trusted Coding mode enabled."
          : "Per-command approval required again.";
      toast(message, "info");
    } catch (err) {
      toast(`Setting failed: ${err instanceof Error ? err.message : String(err)}`, "error");
    }
  };

  // ── Render ──────────────────────────────────────────────────────────────────

  if (authenticated) {
    return (
      <div className="flex h-screen flex-col overflow-hidden">
        <DesktopTitleBar />
        {updateReady && (
          <div
            role="status"
            className="flex items-center justify-between gap-3 bg-[#d4eedc] px-4 py-2 text-sm font-medium text-[#1a4d2e] dark:bg-[#1e3a28] dark:text-[#7ecb99]"
          >
            <span>A new version of Aloe Desktop has been downloaded and is ready to install.</span>
            <button
              onClick={() => void restart()}
              className="rounded-md bg-[#2d7a4f] px-3 py-1 text-xs font-semibold text-white hover:bg-[#246040]"
            >
              Restart now
            </button>
          </div>
        )}
        <div className="relative min-h-0 flex-1 contain-[layout]"><Providers>
          <DesktopRouter desktopPage={
          <main className="relative h-full overflow-y-auto">
            <div className="pointer-events-none absolute right-8 top-8 text-[#6f8747] opacity-[0.05]"><Leaf className="h-52 w-52 rotate-12" /></div>
            <div className="relative mx-auto max-w-6xl px-4 py-8 sm:px-6 lg:px-8 lg:py-10">
              <header className="flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between">
                <div><p className="eyebrow">Aloe Desktop</p><h1 className="mt-2 font-display text-3xl font-semibold text-[#0b3026] dark:text-[#e8f0e0] sm:text-4xl">Desktop controls</h1><p className="mt-2 max-w-2xl text-sm leading-6 text-[#506257] dark:text-[#8aaa90]">Manage the local agent, folder access, command approvals, and recent activity.</p></div>
                <div className={`inline-flex w-fit items-center gap-2 rounded-full border px-3 py-2 text-xs font-semibold ${connected ? "border-[#c9d8b6] bg-[#edf4e7] text-[#4d6c35] dark:border-[#38502e] dark:bg-[#213225] dark:text-[#9fbd7a]" : "border-[#e3c1b8] bg-[#fff1ed] text-[#a65345] dark:border-[#5a3a32] dark:bg-[#33211d] dark:text-[#e8917f]"}`}><span className={`h-2 w-2 rounded-full ${connected ? "bg-[#4f8a47]" : "bg-[#b86d5f]"}`} /><MonitorCheck className="h-4 w-4" />{connected ? "Agent connected" : config.socketStatus || "Disconnected"}</div>
              </header>

              {!connected ? <div className="mt-6 rounded-2xl border border-[#e3c1b8] bg-[#fff4f1] px-4 py-3 text-sm text-[#8a4035] dark:border-[#5a3a32] dark:bg-[#2a1f1c] dark:text-[#e8917f]">The local agent is {config.socketStatus || "disconnected"}.{config.socketError ? ` ${config.socketError}` : ""}</div> : null}
              <div className="mt-8 grid gap-5 lg:grid-cols-2">
                <ConnectionPanel config={config} onReset={() => void resetConnection()} />
                <FoldersPanel folders={config.folders} onAdd={() => void addFolder()} onRemove={(path) => void removeFolder(path)} />
              </div>
              <div className="mt-5 space-y-5"><ApprovalsPanel config={config} pending={pending} onRefresh={() => void refresh()} onApprove={(jobId, approved) => void approve(jobId, approved)} onSetCommandTrustMode={(mode) => void setCommandTrustMode(mode)} /><ActivityList actions={config.recentActions} /></div>
            </div>
            <ToastContainer toasts={toasts} onDismiss={dismiss} />
          </main>
          } />
        </Providers></div>
      </div>
    );
  }

  return (
    <div className="flex h-screen flex-col overflow-hidden">
      <DesktopTitleBar />
      <ThemeProvider attribute="class" defaultTheme="system" enableSystem disableTransitionOnChange>
        <main className="relative flex min-h-0 flex-1 flex-col overflow-y-auto bg-[#f8f5e9] dark:bg-[#0e1a13]">
          {/* Botanical background decorations */}
          <div className="pointer-events-none absolute right-8 top-8 z-0 text-[#6f8747] opacity-[0.06]"><Leaf className="h-28 w-28 rotate-20" /></div>
          <div className="pointer-events-none absolute bottom-14 left-4 z-0 text-[#d98f82] opacity-[0.08]"><ButterflyDecor style={{ width: 80, height: 56 }} /></div>
          <div className="pointer-events-none absolute bottom-10 right-14 z-0 text-[#6f8747] opacity-[0.06]"><Flower2 className="h-16 w-16 rotate-15" /></div>

          <div className="relative z-10 flex items-center gap-3 px-6 pt-8 sm:px-10">
            <div className="brand-mark h-9 w-9"><Leaf className="h-4 w-4" /></div>
            <div>
              <p className="eyebrow">Aloe Desktop</p>
              <h1 className="mt-0.5 font-display text-xl font-semibold text-[#0b3026] dark:text-[#e8f0e0]">Local agent</h1>
            </div>
          </div>

          <div className="relative z-10 flex flex-1 items-center justify-center">
            <AuthScreen setupToken={setupToken} onTokenChange={setSetupToken} onConnect={() => void connect()} />
          </div>

          <ToastContainer toasts={toasts} onDismiss={dismiss} />
        </main>
      </ThemeProvider>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
