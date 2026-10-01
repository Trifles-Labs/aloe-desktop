/* Everything Aloe Desktop adds to the shared web app. The web pages know
   nothing about this machine; they expose slots (see the web app's
   lib/platform.tsx) and this file fills them: the Desktop controls entry, the
   Desktop app settings pane, per-chat folders in the composer, the OAuth
   hand-off to the browser, and the device header that unlocks local tools for
   chats sent from here. */

import { useCallback, useMemo, useRef, useState } from "react";
import { Monitor, MonitorCog } from "lucide-react";

import { configureApiClient } from "@aloe/ui/lib/api";
import type { Platform } from "@aloe/ui/lib/platform";
import type { User } from "@aloe/ui/lib/types";
import ConversationFolders from "./components/ConversationFolders";
import DesktopConnectModal from "./components/DesktopConnectModal";
import { isCodeSession } from "./code/sessions";
import DesktopAppPane from "./settings/DesktopAppPane";
import type { AgentConfig } from "./types";

/* Read on every API call. The backend only registers local file, terminal and
   computer tools for a chat that carries a valid credential for this device. */
let agentCredential: string | null = null;

configureApiClient({
  extraHeaders: (): Record<string, string> => (agentCredential ? { "X-Aloe-Agent": agentCredential } : {}),
  // A rejected user token must not unpair the device; say so instead of
  // redirecting a window that has no /signin page.
  onUnauthorized: () => {
    throw new Error("Your Aloe session was not accepted. Sign out and sign in again.");
  },
});

export function useDesktopPlatform(config: AgentConfig, pendingCount: number, onSignOut: () => void) {
  agentCredential = config.credential;

  /* AuthProvider re-runs its bootstrap whenever these change identity, so they
     stay stable and read the latest values through refs. */
  const profileRef = useRef(config.userProfile);
  profileRef.current = config.userProfile;
  const signOutRef = useRef(onSignOut);
  signOutRef.current = onSignOut;

  const readStoredUser = useCallback(() => (profileRef.current as User | null) ?? null, []);
  const signOut = useCallback(() => signOutRef.current(), []);

  const [connectProvider, setConnectProvider] = useState<string | null>(null);
  const interceptOAuthConnect = useCallback((providerName: string) => {
    setConnectProvider(providerName);
    return true;
  }, []);

  const platform = useMemo<Platform>(
    () => ({
      hostHasBrand: true,
      sidebarItems: [{ href: "/app/desktop", icon: MonitorCog, label: "Desktop controls", count: pendingCount, tone: "clay" }],
      settingsSections: [{ id: "desktop", label: "Desktop app", icon: Monitor, render: () => <DesktopAppPane /> }],
      ComposerAddon: ConversationFolders,
      readStoredUser,
      onSignOut: signOut,
      interceptOAuthConnect,
      // Aloe Code sessions are listed in Code mode, by project.
      hideConversation: isCodeSession,
    }),
    [pendingCount, readStoredUser, signOut, interceptOAuthConnect],
  );

  const overlay = connectProvider ? <DesktopConnectModal providerName={connectProvider} onDismiss={() => setConnectProvider(null)} /> : null;

  return { platform, overlay };
}
