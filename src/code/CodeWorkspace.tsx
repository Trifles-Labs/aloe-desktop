/* Aloe Code: Aloe as a coding agent on a project folder of this computer.
 *
 * It shares the chat thread UI (ChatPageClient) and the device's tools, but is
 * not a chat mode: requests carry `workspace: "code"` and the project path, and
 * the backend answers them with its own tools and system prompt. The composer
 * keeps chat's modes minus Auto, drops per-chat folder sharing and @mentions, and
 * adds plan mode.
 *
 * Three columns: the project and its sessions (ProjectSidebar), the session's chat, and a
 * workbench with what the session changed, the project's files, and the commands it ran. */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { motion } from "framer-motion";
import { Code2, Folder, FolderPlus, GitBranch, Loader2 } from "lucide-react";
import { usePathname, useRouter } from "next/navigation";

import ChatPageClient from "@aloe/ui/components/chat/ChatPageClient";
import LoadingPill from "@aloe/ui/components/ui/LoadingPill";
import Select from "@aloe/ui/components/ui/Select";
import { useAppState } from "@aloe/ui/contexts/AppStateContext";
import { useAuth } from "@aloe/ui/contexts/AuthContext";
import { EASE_OUT } from "@aloe/ui/lib/motion";
import { stripHiddenContext } from "@aloe/ui/lib/utils";
import type { AgentConfig, CommandTrustMode } from "../types";
import { useSessionChanges, useTerminalSessions } from "./bridge";
import ChangesPane from "./ChangesPane";
import CodeHeader from "./CodeHeader";
import FilesPane from "./FilesPane";
import ProjectSidebar, { type SidebarSession } from "./ProjectSidebar";
import TerminalPane from "./TerminalPane";
import Workbench from "./Workbench";
import {
  codeSessionContext,
  codeSessionFor,
  forgetCodeSession,
  projectName,
  readLastProject,
  readWorkbenchPrefs,
  recordCodeSession,
  storeLastCodeRoute,
  storeLastProject,
  storeWorkbenchPrefs,
  useCodeSessions,
  type WorkbenchPrefs,
  type WorkbenchTab,
} from "./sessions";

type Props = {
  config: AgentConfig;
  pendingCount: number;
  /** Opens the OS folder dialog and grants the pick to this device. Resolves to its path, or null if cancelled. */
  onAddProject: () => Promise<string | null>;
};

const TRUST_LABELS: Record<CommandTrustMode, string> = {
  ask: "Asks before commands",
  auto: "Auto-approves safe commands",
  all: "Runs all commands",
};

function routeSessionId(pathname: string): string | null {
  const match = pathname.match(/^\/app\/code\/([^/]+)$/);
  return match ? decodeURIComponent(match[1]) : null;
}

/** The branch of a project, re-read when the project changes or the window regains focus (a checkout elsewhere). */
function useProjectBranch(projectPath: string | null): string | null {
  const [branch, setBranch] = useState<string | null>(null);

  useEffect(() => {
    if (!projectPath) {
      setBranch(null);
      return;
    }
    let cancelled = false;
    const load = () =>
      invoke<string | null>("project_git_branch", { path: projectPath })
        .then((next) => !cancelled && setBranch(next))
        .catch(() => !cancelled && setBranch(null));
    void load();
    window.addEventListener("focus", load);
    return () => {
      cancelled = true;
      window.removeEventListener("focus", load);
    };
  }, [projectPath]);

  return branch;
}

export default function CodeWorkspace({ config, pendingCount, onAddProject }: Props) {
  const router = useRouter();
  const pathname = usePathname();
  const { token, user, isLoading } = useAuth();
  const { conversations, deleteConversation, startDraftConversation } = useAppState();
  const sessions = useCodeSessions();

  const sessionId = routeSessionId(pathname);
  const projects = config.folders;
  const isGranted = useCallback((path: string) => projects.some((folder) => folder.path === path), [projects]);

  /* The project a new session will open in: the last one used, while it is still granted. */
  const [draftProject, setDraftProject] = useState<string | null>(() => readLastProject());
  useEffect(() => {
    if (draftProject && isGranted(draftProject)) return;
    setDraftProject(projects[0]?.path ?? null);
  }, [draftProject, isGranted, projects]);

  const activeSession = sessionId ? sessions[sessionId] ?? null : null;
  /* A session the server knows is Code but this device has no project for: started on
     another computer, or before a reinstall cleared local storage. Its project is a
     path on some disk, so it has to be chosen here rather than guessed. */
  const unassigned = Boolean(sessionId && !activeSession && conversations.some((conversation) => conversation.id === sessionId && conversation.workspace === "code"));
  const activeProject = activeSession?.projectPath ?? (unassigned ? null : draftProject);
  const projectReady = Boolean(activeProject && isGranted(activeProject));
  const branch = useProjectBranch(projectReady ? activeProject : null);

  /* Read by onConversationId, which fires inside ChatPageClient's effects; a ref
     keeps the callback stable so those effects don't re-run on every render. */
  const activeProjectRef = useRef(activeProject);
  activeProjectRef.current = activeProject;

  const conversationsRef = useRef(conversations);
  conversationsRef.current = conversations;

  const sessionIdRef = useRef(sessionId);
  sessionIdRef.current = sessionId;

  /* Only two ids ever get a project here: a session's first id, sent from the
     new-session screen (no session in the URL yet), and the server id that later
     replaces it, which inherits the same project. Opening an existing session
     also reports its id, and must never be handed whatever project is selected. */
  const handleConversationId = useCallback((conversationId: string) => {
    if (codeSessionFor(conversationId)) return;
    const draftId = conversationsRef.current.find((conversation) => conversation.id === conversationId)?.clientId;
    const inherited = draftId ? codeSessionFor(draftId) : null;
    if (inherited) {
      recordCodeSession(conversationId, inherited.projectPath);
      // The draft id points at nothing once the server's id has replaced it.
      if (draftId !== conversationId) forgetCodeSession(draftId!);
      return;
    }
    const project = activeProjectRef.current;
    if (!sessionIdRef.current && project) recordCodeSession(conversationId, project);
  }, []);

  useEffect(() => {
    storeLastCodeRoute(pathname);
  }, [pathname]);

  useEffect(() => {
    if (!isLoading && !token) router.replace("/signin");
  }, [isLoading, router, token]);

  useEffect(() => {
    if (!isLoading && user && !user.onboardedAt) router.replace("/app/onboarding");
  }, [isLoading, router, user]);

  const chooseProject = (path: string) => {
    setDraftProject(path);
    storeLastProject(path);
  };

  const [addingProject, setAddingProject] = useState(false);
  const addProject = async () => {
    if (addingProject) return;
    setAddingProject(true);
    try {
      const path = await onAddProject();
      if (path) chooseProject(path);
    } finally {
      setAddingProject(false);
    }
  };

  const openNewSession = useCallback(
    (project?: string) => {
      if (project) chooseProject(project);
      startDraftConversation();
      router.push("/app/code");
    },
    [router, startDraftConversation],
  );

  /* Switching project in the sidebar while a session of another project is open starts fresh
     in the new one; staying on the old session would leave the sidebar and the chat disagreeing. */
  const switchProject = (path: string) => {
    if (activeSession && activeSession.projectPath !== path) openNewSession(path);
    else chooseProject(path);
  };

  /* The title bar's "New" button means a new session while this mode is showing. */
  useEffect(() => {
    const onNew = () => openNewSession();
    window.addEventListener("aloe:new-code-session", onNew);
    return () => window.removeEventListener("aloe:new-code-session", onNew);
  }, [openNewSession]);

  const removeSession = async (conversationId: string) => {
    try {
      await deleteConversation(conversationId);
    } catch {
      // Already gone server-side, or offline: still drop it from this list.
    }
    forgetCodeSession(conversationId);
    if (conversationId === sessionId) router.replace("/app/code");
  };

  /* The sidebar's two lists: this project's sessions, and sessions with no project on this computer. */
  const { projectSessions, unassignedSessions } = useMemo(() => {
    const inProject: SidebarSession[] = [];
    const orphans: SidebarSession[] = [];
    for (const conversation of conversations) {
      const session = sessions[conversation.id];
      if (!session && conversation.workspace !== "code") continue;
      const item = { id: conversation.id, title: stripHiddenContext(conversation.title) || "New session", createdAt: conversation.createdAt };
      if (!session) orphans.push(item);
      else if (session.projectPath === activeProject) inProject.push(item);
    }
    const newestFirst = (a: SidebarSession, b: SidebarSession) => b.createdAt.getTime() - a.createdAt.getTime();
    return { projectSessions: inProject.sort(newestFirst), unassignedSessions: orphans.sort(newestFirst) };
  }, [activeProject, conversations, sessions]);

  /* The workbench, remembered per project. */
  const [workbench, setWorkbench] = useState<WorkbenchPrefs>(() => readWorkbenchPrefs(activeProject));
  useEffect(() => {
    setWorkbench(readWorkbenchPrefs(activeProject));
  }, [activeProject]);
  const updateWorkbench = (next: WorkbenchPrefs) => {
    setWorkbench(next);
    if (activeProject) storeWorkbenchPrefs(activeProject, next);
  };
  const toggleTab = (tab: WorkbenchTab) => {
    updateWorkbench(workbench.open && workbench.tab === tab ? { ...workbench, open: false } : { ...workbench, open: true, tab });
  };

  const paneSession = projectReady ? sessionId : null;
  const changes = useSessionChanges(paneSession, projectReady ? activeProject : null);
  const terminals = useTerminalSessions(paneSession);
  const changedPaths = useMemo(() => new Set(changes.files.map((file) => file.path)), [changes.files]);
  const runningCount = terminals.sessions.filter((session) => session.status.state === "running").length;

  const [openRequest, setOpenRequest] = useState<{ path: string; nonce: number } | null>(null);
  const openInFiles = (path: string) => {
    setOpenRequest((current) => ({ path, nonce: (current?.nonce ?? 0) + 1 }));
    updateWorkbench({ ...workbench, open: true, tab: "files" });
  };

  if (isLoading || !token) {
    return (
      <main className="app-wash flex h-full items-center justify-center">
        <LoadingPill label="Loading Aloe Code..." />
      </main>
    );
  }

  const sendBlockedReason = unassigned
    ? "Choose which project on this computer this session works in."
    : !activeProject
    ? "Add a project folder to start a session — Aloe Code works inside a folder on this computer."
    : !isGranted(activeProject)
      ? `${projectName(activeProject)} is no longer granted to Aloe. Add it again as a project to continue this session.`
      : null;

  const projectOptions = projects.map((folder) => ({ value: folder.path, label: folder.label ?? projectName(folder.path), description: folder.path }));

  const emptyState = (
    <div className="flex w-full flex-col items-center">
      <motion.span
        initial={{ opacity: 0, scale: 0.94 }}
        animate={{ opacity: 1, scale: 1 }}
        transition={{ duration: 0.3, ease: EASE_OUT }}
        className="brand-mark h-11 w-11"
      >
        <Code2 className="h-5 w-5" />
      </motion.span>
      {projects.length === 0 ? (
        <>
          <h1 className="font-display mt-4 text-3xl font-semibold tracking-tight text-balance text-ink sm:text-4xl">Choose a project to start</h1>
          <p className="mt-2 max-w-md text-sm leading-6 text-ink-soft">
            Aloe Code works inside one folder on this computer. It reads, edits and runs code there, with the same approvals as everything else.
          </p>
          <button
            type="button"
            onClick={() => void addProject()}
            disabled={addingProject}
            className="press-tap mt-6 inline-flex h-10 items-center gap-2 rounded-lg bg-moss px-4 text-sm font-semibold text-on-accent hover:bg-moss/90 disabled:opacity-60"
          >
            {addingProject ? <Loader2 className="h-4 w-4 animate-spin" /> : <FolderPlus className="h-4 w-4" />}
            Choose a project folder
          </button>
        </>
      ) : (
        <>
          <motion.h1
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.4, ease: EASE_OUT, delay: 0.06 }}
            className="font-display mt-4 text-3xl font-semibold tracking-tight text-balance text-ink sm:text-4xl"
          >
            What should we build?
          </motion.h1>
          {draftProject ? (
            <p className="mt-3 inline-flex flex-wrap items-center justify-center gap-x-3 gap-y-1 text-sm text-ink-soft">
              <span className="inline-flex items-center gap-1.5 font-medium text-ink">
                <Folder className="h-4 w-4 text-moss" />
                {projectName(draftProject)}
              </span>
              {branch ? (
                <span className="inline-flex items-center gap-1 font-mono text-[11px]">
                  <GitBranch className="h-3 w-3" />
                  {branch}
                </span>
              ) : null}
            </p>
          ) : null}
          {/* The sidebar holds the project switcher; below md it is hidden, so offer it here. */}
          {draftProject ? (
            <Select className="mt-4 w-full max-w-xs md:hidden" value={draftProject} options={projectOptions} onChange={chooseProject} />
          ) : null}
        </>
      )}
    </div>
  );

  const header = unassigned && sessionId ? (
    <div className="flex items-center gap-3 border-b border-edge px-4 py-2 text-xs text-ink-soft sm:px-6">
      <Folder className="h-3.5 w-3.5 shrink-0 text-clay" />
      <span>This session has no project on this computer.</span>
      <span className="flex-1" />
      {projects.length > 0 ? (
        <Select
          className="w-56"
          value=""
          placeholder="Choose its project"
          options={projectOptions}
          menuAlign="right"
          onChange={(path) => recordCodeSession(sessionId, path)}
        />
      ) : (
        <button type="button" onClick={() => void addProject()} className="font-semibold text-moss hover:underline">
          Add a project folder
        </button>
      )}
    </div>
  ) : activeProject ? (
    <CodeHeader
      project={activeProject}
      branch={branch}
      pendingCount={pendingCount}
      trustLabel={TRUST_LABELS[config.commandTrustMode] ?? TRUST_LABELS.auto}
      openTab={workbench.open && projectReady ? workbench.tab : null}
      changeCount={changes.files.length}
      runningCount={runningCount}
      onToggleTab={toggleTab}
    />
  ) : null;

  return (
    <div className="app-wash flex h-full overflow-hidden text-ink">
      <ProjectSidebar
        projectOptions={projectOptions}
        project={activeProject && isGranted(activeProject) ? activeProject : draftProject}
        onChooseProject={switchProject}
        onAddProject={() => void addProject()}
        addingProject={addingProject}
        sessions={projectSessions}
        unassigned={unassignedSessions}
        activeSessionId={sessionId}
        onNewSession={() => openNewSession()}
        onDeleteSession={(id) => void removeSession(id)}
        onAssignSession={recordCodeSession}
      />

      <main className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <ChatPageClient
          routeBase="/app/code"
          workspace="code"
          messageContext={activeProject ? codeSessionContext(branch) : null}
          projectPath={activeProject}
          emptyState={emptyState}
          header={header}
          onConversationId={handleConversationId}
          sendBlockedReason={sendBlockedReason}
        />
      </main>

      {workbench.open && projectReady ? (
        <Workbench
          prefs={workbench}
          onChange={updateWorkbench}
          panes={{
            changes: (
              <ChangesPane conversationId={paneSession} files={changes.files} error={changes.error} onRefresh={() => void changes.refresh()} onOpenFile={openInFiles} />
            ),
            files: <FilesPane projectPath={activeProject} conversationId={paneSession} changedPaths={changedPaths} openRequest={openRequest} />,
            terminal: <TerminalPane conversationId={paneSession} sessions={terminals.sessions} onRefresh={() => void terminals.refresh()} />,
          }}
        />
      ) : null}
    </div>
  );
}
