import React, { useCallback, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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
import Providers from "@aloe/ui/providers";
import AppLayout from "@aloe/ui/pages/AppLayout";
import ChatSurfaceLayout from "@aloe/ui/pages/ChatSurfaceLayout";
import ConversationsPage from "@aloe/ui/pages/ConversationsPage";
import OnboardingPage from "@aloe/ui/pages/OnboardingPage";
import PlansPage from "@aloe/ui/pages/PlansPage";
import SettingsPage from "@aloe/ui/pages/SettingsPage";
import TasksPage from "@aloe/ui/pages/TasksPage";
import UsagePage from "@aloe/ui/pages/UsagePage";
import BoardPage from "@aloe/ui/pages/BoardPage";
import ApprovalsPage from "@aloe/ui/pages/ApprovalsPage";
import MemoryPage from "@aloe/ui/pages/MemoryPage";
import { PlatformProvider } from "@aloe/ui/lib/platform";
import { usePathname, useRouter } from "next/navigation";
import { navigateTo } from "./shims/next-navigation";
import { DEFAULT_CONFIG } from "./types";
import type { AgentConfig, CommandTrustMode, PendingApproval } from "./types";
import { errorMessage, GOOGLE_AUTH_EVENT, mintAgentSetupToken, startGoogleAuth } from "./lib/desktop";
import { useDesktopPlatform } from "./platform";
import "./web.css";

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
    /* Home and chat are one screen: the surface layout is the whole UI (it reads
       the conversation id from the URL itself). */
    "/app/home": <ChatSurfaceLayout>{null}</ChatSurfaceLayout>,
    "/app/chat": <ChatSurfaceLayout>{null}</ChatSurfaceLayout>,
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
  const [googleConnecting, setGoogleConnecting] = useState(false);
  const [authError, setAuthError] = useState<string | null>(null);
  const { toasts, toast, dismiss, pause, resume } = useToasts();
  const { updateReady, restart } = useAutoUpdate();

  const authenticated = Boolean(config.agentId && config.credential && config.userToken);

  /* The deep-link listener lives for the whole window lifetime; this ref lets it
     ignore a token that arrives after the device is already paired (e.g. a stale
     browser tab finishing a flow from a previous run). */
  const authenticatedRef = useRef(authenticated);
  authenticatedRef.current = authenticated;

  /* Google tokens are handled once. The browser tab hands the token through
     either the Tauri event (app already open) or the pending-token command
     (app launched by the deep link) — the set keeps the two paths from
     pairing twice. */
  const handledGoogleTokens = useRef(new Set<string>());

  /* The shared web pages read the session token from localStorage, as they do in
     a browser. The profile reaches them through the platform instead. */
  const persistUserToken = (nextConfig: AgentConfig) => {
    if (nextConfig.userToken) {
      window.localStorage.setItem("aloe_token", nextConfig.userToken);
    } else {
      window.localStorage.removeItem("aloe_token");
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

  /* Signing out of the shared app unpairs this device; the setup screen takes over. */
  const signOut = useCallback(() => {
    navigateTo("/signin", true);
    void invoke<AgentConfig>("reset_agent_connection").then(setConfig).catch((error) => {
      toast(`Logout failed: ${errorMessage(error)}`, "error");
    });
  }, [toast]);

  const { platform, overlay } = useDesktopPlatform(config, pending.length, signOut);

  // ── Handlers ────────────────────────────────────────────────────────────────

  const googleSignIn = async () => {
    setGoogleConnecting(true);
    setAuthError(null);
    try {
      // The browser opens, Google runs there, and the finished session comes
      // back through the `aloe://` deep link — completeGoogleAuth finishes the
      // pairing, so the button stays "in progress" until then (or a timeout).
      await startGoogleAuth();
    } catch (err) {
      setGoogleConnecting(false);
      setAuthError(errorMessage(err));
    }
  };

  /* The session token from the browser is only a login — this device still needs an
     agent credential. Minting a setup token from the signed-in session and registering
     with it does exactly what pasting a token from the web app would have done. */
  const completeGoogleAuth = useCallback(
    async (token: string) => {
      if (handledGoogleTokens.current.has(token) || authenticatedRef.current) return;
      handledGoogleTokens.current.add(token);
      try {
        const setupToken = await mintAgentSetupToken(token);
        const next = await invoke<AgentConfig>("register_agent", { token: setupToken });
        persistUserToken(next);
        setConfig(next);
        toast("Signed in with Google — this device is paired.", "success");
      } catch (err) {
        toast(`Google sign in failed: ${errorMessage(err)}`, "error");
        setAuthError(errorMessage(err));
      } finally {
        setGoogleConnecting(false);
      }
    },
    [toast],
  );

  /* Catches the Google session handed back by the browser: emitted while the app is
     already open, or drained from Rust when the app was launched by the deep link. */
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    void (async () => {
      unlisten = await listen<string>(GOOGLE_AUTH_EVENT, (event) => {
        void completeGoogleAuth(event.payload);
      });
      if (cancelled) return;
      const pending = await invoke<string | null>("take_pending_oauth_token");
      if (pending) void completeGoogleAuth(pending);
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [completeGoogleAuth]);

  /* If the browser tab is abandoned mid-flow, the Google button should not stay
     spinning forever — it reverts, and a later deep link still completes anyway. */
  useEffect(() => {
    if (!googleConnecting) return;
    const timer = window.setTimeout(() => setGoogleConnecting(false), 8 * 60_000);
    return () => window.clearTimeout(timer);
  }, [googleConnecting]);

  const resetConnection = async () => {
    try {
      const next = await invoke<AgentConfig>("reset_agent_connection");
      setConfig(next);
      toast("Logged out. Sign in again to reconnect this computer.", "info");
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

  const setDesktopControl = async (enabled: boolean) => {
    try {
      const next = await invoke<AgentConfig>("set_desktop_control_enabled", { enabled });
      setConfig(next);
      toast(enabled ? "Aloe can now control this computer." : "Desktop control turned off.", "info");
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
          <PlatformProvider value={platform}>
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
                    onSetDesktopControl={(enabled) => void setDesktopControl(enabled)}
                  />
                }
              />
              {overlay}
            </Providers>
          </PlatformProvider>
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
              onGoogleSignIn={() => void googleSignIn()}
              googleConnecting={googleConnecting}
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
