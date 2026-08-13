import React, { useCallback, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { MotionConfig } from "framer-motion";
import { Leaf } from "lucide-react";

import { useToasts, ToastContainer } from "./toast";
import { useAutoUpdate } from "./hooks/useAutoUpdate";
import { ButterflyDecor } from "./components/ButterflyDecor";
import { AuthScreen } from "./components/AuthScreen";
import { DesktopControls } from "./components/DesktopControls";
import { DesktopTitleBar } from "./components/DesktopTitleBar";
import { UpdateBanner } from "./components/UpdateBanner";
import { ThemeProvider } from "next-themes";
import Providers from "@/app/providers";
import AppLayout from "@/app/(app)/layout";
import ChatSurfaceLayout from "@/app/(app)/app/(surface)/layout";
import ChatPage from "@/app/(app)/app/(surface)/chat/page";
import ConversationsPage from "@/app/(app)/app/conversations/page";
import HomePage from "@/app/(app)/app/(surface)/home/page";
import OnboardingPage from "@/app/(app)/app/onboarding/page";
import PlansPage from "@/app/(app)/app/plans/page";
import SettingsPage from "@/app/(app)/app/settings/page";
import TasksPage from "@/app/(app)/app/tasks/page";
import UsagePage from "@/app/(app)/app/usage/page";
import BoardPage from "@/app/(app)/app/board/page";
import ApprovalsPage from "@/app/(app)/app/approvals/page";
import MemoryPage from "@/app/(app)/app/memory/page";
import { usePathname, useRouter } from "next/navigation";
import { DEFAULT_CONFIG } from "./types";
import type { AgentConfig, CommandTrustMode, PendingApproval } from "./types";
import { errorMessage } from "./lib/desktop";
import "./web.css";

type DesktopPreferences = { runOnStartup: boolean; startMinimized: boolean };
const preferenceShape = (config: AgentConfig): DesktopPreferences => ({ runOnStartup: config.runOnStartup, startMinimized: config.startMinimized });
(window as Window & { __ALOE_DESKTOP__?: unknown }).__ALOE_DESKTOP__ = {
  getPreferences: async () => preferenceShape(await invoke<AgentConfig>("get_config")),
  setRunOnStartup: async (enabled: boolean) => preferenceShape(await invoke<AgentConfig>("set_run_on_startup", { enabled })),
  setStartMinimized: async (enabled: boolean) => preferenceShape(await invoke<AgentConfig>("set_start_minimized", { enabled })),
  openExternal: (url: string) => invoke<void>("open_external_url", { url }),
};

/* Routes the web app answers with a redirect. Without them here, a link to
   Integrations or MCP quietly landed on the chat page — the desktop router
   falls back to chat for anything it doesn't know. */
const ROUTE_ALIASES: Record<string, string> = {
  "/app": "/app/home",
  "/app/integrations": "/app/settings?section=connections",
  "/app/mcp": "/app/settings?section=connections",
  "/app/mobile-login": "/app/settings?section=devices",
};

function DesktopRouter({ desktopPage }: { desktopPage: React.ReactNode }) {
  const pathname = usePathname();
  const router = useRouter();

  useEffect(() => {
    const alias = ROUTE_ALIASES[pathname];
    if (alias) router.replace(alias);
  }, [pathname, router]);

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
    /* First run sends people here before anything else is usable; without the
       route the wizard was skipped and you landed in an empty chat. */
    "/app/onboarding": <OnboardingPage />,
    "/app/desktop": desktopPage,
  };

  // Unknown paths — including /app/chat/<id>, which the page reads from the
  // URL itself — land on the chat surface.
  const page = pages[pathname] ?? pages["/app/chat"];
  return <AppLayout>{page}</AppLayout>;
}

function App() {
  const [config, setConfig] = useState<AgentConfig>(DEFAULT_CONFIG);
  const [pending, setPending] = useState<PendingApproval[]>([]);
  const [setupToken, setSetupToken] = useState("");
  const [connecting, setConnecting] = useState(false);
  const [authError, setAuthError] = useState<string | null>(null);
  const { toasts, toast, dismiss, pause, resume } = useToasts();
  const { updateReady, restart } = useAutoUpdate();

  const authenticated = Boolean(config.agentId && config.credential && config.userToken);

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
    try {
      const [nextConfig, nextPending] = await Promise.all([
        invoke<AgentConfig>("get_config"),
        invoke<PendingApproval[]>("get_pending_approvals"),
      ]);
      persistUserToken(nextConfig);
      setConfig(nextConfig);
      setPending(nextPending);
    } catch {
      // A dropped poll is not news — the next tick either recovers or the
      // socket status on screen already says the agent is unreachable.
    }
  }, []);

  /* Polling stops while the window is hidden in the tray and resumes with an
     immediate read, so a minimised app isn't waking the agent twice a second
     to answer questions nobody is looking at. */
  useEffect(() => {
    let timer = 0;

    const stop = () => {
      if (timer) window.clearInterval(timer);
      timer = 0;
    };

    const start = () => {
      stop();
      void refresh();
      timer = window.setInterval(() => void refresh(), 1500);
    };

    const onVisibility = () => (document.hidden ? stop() : start());

    start();
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("focus", onVisibility);
    return () => {
      stop();
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("focus", onVisibility);
    };
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
        toast(`Logout failed: ${errorMessage(error)}`, "error");
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
    setConnecting(true);
    setAuthError(null);
    try {
      const next = await invoke<AgentConfig>("register_agent", { token: setupToken.trim() });
      persistUserToken(next);
      setSetupToken("");
      setConfig(next);
      toast("This device is paired — opening the socket connection.", "success");
    } catch (err) {
      // Stays on the screen next to the field rather than in a toast that has
      // timed out by the time you look back at what you pasted.
      setAuthError(errorMessage(err));
    } finally {
      setConnecting(false);
    }
  };

  const resetConnection = async () => {
    try {
      const next = await invoke<AgentConfig>("reset_agent_connection");
      setConfig(next);
      toast("Logged out. Paste a fresh setup token to reconnect.", "info");
    } catch (err) {
      toast(`Reset failed: ${errorMessage(err)}`, "error");
    }
  };

  const addFolder = async () => {
    try {
      const next = await invoke<AgentConfig>("add_folder");
      const added = next.folders[next.folders.length - 1];
      setConfig(next);
      if (added) toast(`Folder granted: ${added.label ?? added.path}`, "success");
    } catch (err) {
      toast(`Could not add folder: ${errorMessage(err)}`, "error");
    }
  };

  const removeFolder = async (path: string) => {
    try {
      const next = await invoke<AgentConfig>("remove_folder", { path });
      setConfig(next);
      toast("Folder removed.", "info");
    } catch (err) {
      toast(`Could not remove folder: ${errorMessage(err)}`, "error");
    }
  };

  const setCommandTrustMode = async (mode: CommandTrustMode) => {
    try {
      const next = await invoke<AgentConfig>("set_command_trust_mode", { mode });
      setConfig(next);
      const message = mode === "all"
        ? "All command approvals disabled."
        : mode === "auto"
          ? "Auto mode enabled."
          : "Per-command approval required again.";
      toast(message, "info");
    } catch (err) {
      toast(`Setting failed: ${errorMessage(err)}`, "error");
    }
  };

  // ── Render ──────────────────────────────────────────────────────────────────

  const toastLayer = <ToastContainer toasts={toasts} onDismiss={dismiss} onPause={pause} onResume={resume} />;

  if (authenticated) {
    return (
      <div className="flex h-screen flex-col overflow-hidden">
        <DesktopTitleBar />
        {updateReady && <UpdateBanner onRestart={() => void restart()} />}
        <div className="relative min-h-0 flex-1 contain-[layout]">
          <Providers>
            <DesktopRouter
              desktopPage={
                <DesktopControls
                  config={config}
                  pending={pending}
                  onRefresh={() => void refresh()}
                  onReset={() => void resetConnection()}
                  onAddFolder={() => void addFolder()}
                  onRemoveFolder={(path) => void removeFolder(path)}
                  onSetCommandTrustMode={(mode) => void setCommandTrustMode(mode)}
                />
              }
            />
          </Providers>
          {toastLayer}
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-screen flex-col overflow-hidden">
      <DesktopTitleBar />
      <ThemeProvider attribute="class" defaultTheme="system" enableSystem disableTransitionOnChange>
        <main className="relative flex min-h-0 flex-1 flex-col overflow-y-auto bg-canvas">
          {/* One watermark, not three. It drifts slowly enough to read as light
              moving rather than as motion competing with the form. */}
          <div className="pointer-events-none absolute -right-6 top-6 z-0 text-moss opacity-[0.05] drift-slow">
            <Leaf className="h-56 w-56 rotate-12" />
          </div>
          <div className="pointer-events-none absolute bottom-10 left-6 z-0 text-blush opacity-[0.06]">
            <ButterflyDecor style={{ width: 72, height: 50 }} />
          </div>

          <div className="relative z-10 flex flex-1 items-center justify-center">
            <AuthScreen
              setupToken={setupToken}
              onTokenChange={(value) => { setSetupToken(value); if (authError) setAuthError(null); }}
              onConnect={() => void connect()}
              connecting={connecting}
              error={authError}
            />
          </div>

          {toastLayer}
        </main>
      </ThemeProvider>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {/* Every framer-motion animation below this respects the OS reduced-motion
        setting, including the title bar and setup screen that sit outside the
        web app's own Providers. */}
    <MotionConfig reducedMotion="user">
      <App />
    </MotionConfig>
  </React.StrictMode>,
);
