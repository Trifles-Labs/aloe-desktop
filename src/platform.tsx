/* Everything Aloe Desktop adds to the shared web app. The web pages know
   nothing about this machine; they expose slots (see the web app's
   lib/platform.tsx) and this file fills them: the Desktop app settings pane
   (startup, connection, folders, approvals, desktop control, activity), per-chat folders in the composer, the OAuth
   hand-off to the browser, and the device header that unlocks local tools for
   chats sent from here. */

import { useCallback, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Monitor } from "lucide-react";

import { configureApiClient } from "@aloe/ui/lib/api";
import { isLocalChatId } from "@aloe/ui/lib/local_code_chats";
import type { Platform } from "@aloe/ui/lib/platform";
import type { LocalSkillSummary, User } from "@aloe/ui/lib/types";
import DesktopConnectModal from "./components/DesktopConnectModal";
import { localCodeStore } from "./code/chatStore";
import { isCodeSession } from "./code/sessions";
import DesktopAppPane from "./settings/DesktopAppPane";
import type { AgentConfig } from "./types";

/* Read on every API call. The backend only registers local file, terminal and
   computer tools for a chat that carries a valid credential for this device. */
let agentCredential: string | null = null;

configureApiClient({
  extraHeaders: (): Record<string, string> => (agentCredential ? { "X-Aloe-Agent": agentCredential } : {}),
  // A rejected user token must not sign the device out; say so instead of
  // redirecting a window that has no /signin page.
  onUnauthorized: () => {
    throw new Error("Your Aloe session was not accepted. Sign out and sign in again.");
  },
});

export function useDesktopPlatform(config: AgentConfig, onSignOut: () => void) {
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
      kind: "desktop",
      hostHasBrand: true,
      settingsSections: [{ id: "desktop", label: "Desktop app", icon: Monitor, render: () => <DesktopAppPane /> }],
      readStoredUser,
      onSignOut: signOut,
      interceptOAuthConnect,
      // Aloe Code sessions are listed in Code mode, by project.
      hideConversation: (id) => isCodeSession(id) || isLocalChatId(id),
      // ...and kept only on this computer; the server stores none of them.
      codeStore: localCodeStore,
      // Files Aloe shared in chat, opened by the id this computer minted (shared_files.rs).
      openSharedFile: (shareId, { reveal }) => invoke<void>("shared_file_open", { id: shareId, reveal }),
      // Skills in ~/.agents/skills and Claude Code's and Codex's skill folders (local_skills.rs).
      localSkills: {
        list: () => invoke<LocalSkillSummary[]>("local_skills_list"),
        refresh: () => invoke<LocalSkillSummary[]>("local_skills_refresh"),
        openFolder: () => invoke<void>("local_skills_open_folder"),
      },
    }),
    [readStoredUser, signOut, interceptOAuthConnect],
  );

  const overlay = connectProvider ? <DesktopConnectModal providerName={connectProvider} onDismiss={() => setConnectProvider(null)} /> : null;

  return { platform, overlay };
}
