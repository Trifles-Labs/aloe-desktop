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
import ChatSurfaceLayout from "@/app/(app)/app/(surface)/layout";
import ChatPage from "@/app/(app)/app/(surface)/chat/page";
import ConversationsPage from "@/app/(app)/app/conversations/page";
import HomePage from "@/app/(app)/app/(surface)/home/page";
import IntegrationsPage from "@/app/(app)/app/integrations/page";
import MobileLoginPage from "@/app/(app)/app/mobile-login/page";
import PlansPage from "@/app/(app)/app/plans/page";
import SettingsPage from "@/app/(app)/app/settings/page";
import TasksPage from "@/app/(app)/app/tasks/page";
import UsagePage from "@/app/(app)/app/usage/page";
import BoardPage from "@/app/(app)/app/board/page";
import ApprovalsPage from "@/app/(app)/app/approvals/page";
import MemoryPage from "@/app/(app)/app/memory/page";
import McpPage from "@/app/(app)/app/mcp/page";
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
    "/app/home": <ChatSurfaceLayout><HomePage /></ChatSurfaceLayout>,
    "/app/chat": <ChatSurfaceLayout><ChatPage /></ChatSurfaceLayout>,
    "/app/conversations": <ConversationsPage />,
    "/app/plans": <PlansPage />,
    "/app/settings": <SettingsPage />,
    "/app/tasks": <TasksPage />,
    "/app/usage": <UsagePage />,
    "/app/board": <BoardPage />,
    "/app/approvals": <ApprovalsPage />,
    "/app/memory": <MemoryPage />,
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
            className="flex items-center justify-between gap-3 bg-sage-soft px-4 py-2 text-sm font-medium text-moss"
          >
            <span>A new version of Aloe Desktop has been downloaded and is ready to install.</span>
            <button
              onClick={() => void restart()}
              className="rounded-md bg-pine px-3 py-1 text-xs font-semibold text-cream hover:bg-pine-hover"
            >
              Restart now
            </button>
          </div>
        )}
        <div className="relative min-h-0 flex-1 contain-[layout]"><Providers>
          <DesktopRouter desktopPage={
          <main className="relative h-full overflow-y-auto">
            <div className="pointer-events-none absolute right-8 top-8 text-moss opacity-[0.05]"><Leaf className="h-52 w-52 rotate-12" /></div>
            <div className="relative mx-auto max-w-6xl px-4 py-8 sm:px-6 lg:px-8 lg:py-10">
              <header className="flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between">
                <div><p className="eyebrow">Aloe Desktop</p><h1 className="mt-2 font-display text-3xl font-semibold text-ink sm:text-4xl">Desktop controls</h1><p className="mt-2 max-w-2xl text-sm leading-6 text-ink-soft">Manage the local agent, folder access, command approvals, and recent activity.</p></div>
                <div className={`inline-flex w-fit items-center gap-2 rounded-full px-3 py-1 text-xs font-semibold backdrop-blur-sm ${connected ? "bg-sage/80 text-ink" : "bg-black/4 dark:bg-white/6 text-ink-soft/80"}`}><span className={`h-2 w-2 rounded-full ${connected ? "bg-moss watch-pulse" : "bg-ink-soft/40"}`} /><MonitorCheck className="h-4 w-4" />{connected ? "Agent connected" : config.socketStatus || "Disconnected"}</div>
              </header>

              {!connected ? <div className="mt-6 rounded-2xl border border-clay/40 bg-clay/8 px-4 py-3 text-sm text-danger">The local agent is {config.socketStatus || "disconnected"}.{config.socketError ? ` ${config.socketError}` : ""}</div> : null}
              <div className="mt-8 grid gap-5 lg:grid-cols-2">
                <ConnectionPanel config={config} onReset={() => void resetConnection()} />
                <FoldersPanel folders={config.folders} onAdd={() => void addFolder()} onRemove={(path) => void removeFolder(path)} />
              </div>
              <div className="mt-5 space-y-5"><ApprovalsPanel config={config} pending={pending} onRefresh={() => void refresh()} onSetCommandTrustMode={(mode) => void setCommandTrustMode(mode)} /><ActivityList actions={config.recentActions} /></div>
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
        <main className="relative flex min-h-0 flex-1 flex-col overflow-y-auto bg-canvas">
          {/* Botanical background decorations */}
          <div className="pointer-events-none absolute right-8 top-8 z-0 text-moss opacity-[0.06]"><Leaf className="h-28 w-28 rotate-20" /></div>
          <div className="pointer-events-none absolute bottom-14 left-4 z-0 text-blush opacity-[0.08]"><ButterflyDecor style={{ width: 80, height: 56 }} /></div>
          <div className="pointer-events-none absolute bottom-10 right-14 z-0 text-moss opacity-[0.06]"><Flower2 className="h-16 w-16 rotate-15" /></div>

          <div className="relative z-10 flex items-center gap-3 px-6 pt-8 sm:px-10">
            <div className="brand-mark h-9 w-9"><Leaf className="h-4 w-4" /></div>
            <div>
              <p className="eyebrow">Aloe Desktop</p>
              <h1 className="mt-0.5 font-display text-xl font-semibold text-ink">Local agent</h1>
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
