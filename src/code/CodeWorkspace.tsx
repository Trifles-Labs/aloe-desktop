/* Aloe Code: Aloe as a coding agent on a project folder of this computer.
 *
 * Same engine as chat — the shared ChatPageClient, the backend's agent loop,
 * and the file, search and terminal tools this device already runs for chats
 * sent from here. What makes it a separate mode is the frame around it: every
 * session belongs to one project folder, sessions are listed by project, and
 * each message tells the model which folder it is working in (as hidden
 * context, so the thread shows only what the user typed). */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AnimatePresence, motion } from "framer-motion";
import { Code2, Folder, FolderPlus, GitBranch, Loader2, Plus, ShieldAlert, Trash2 } from "lucide-react";
import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";

import ChatPageClient from "@aloe/ui/components/chat/ChatPageClient";
import LoadingPill from "@aloe/ui/components/ui/LoadingPill";
import Select from "@aloe/ui/components/ui/Select";
import { useAppState } from "@aloe/ui/contexts/AppStateContext";
import { useAuth } from "@aloe/ui/contexts/AuthContext";
import { EASE_OUT } from "@aloe/ui/lib/motion";
import { cn, relativeTime, stripHiddenContext } from "@aloe/ui/lib/utils";
import type { AgentConfig, CommandTrustMode } from "../types";
import {
  codeSessionContext,
  codeSessionFor,
  forgetCodeSession,
  projectName,
  readLastProject,
  recordCodeSession,
  storeLastCodeRoute,
  storeLastProject,
  useCodeSessions,
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
  const branch = useProjectBranch(activeProject && isGranted(activeProject) ? activeProject : null);

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

  /* Sessions by project, newest first. Projects with no sessions still get a
     heading, so a freshly added folder is visible and one click from a session. */
  const groups = useMemo(() => {
    const UNASSIGNED = "";
    const byProject = new Map<string, Array<{ id: string; title: string; createdAt: Date }>>();
    for (const folder of projects) byProject.set(folder.path, []);
    for (const conversation of conversations) {
      const session = sessions[conversation.id];
      if (!session && conversation.workspace !== "code") continue;
      const key = session?.projectPath ?? UNASSIGNED;
      const list = byProject.get(key) ?? [];
      list.push({ id: conversation.id, title: stripHiddenContext(conversation.title) || "New session", createdAt: conversation.createdAt });
      byProject.set(key, list);
    }
    return [...byProject.entries()]
      .map(([path, items]) => ({
        path,
        label: path === UNASSIGNED ? "No project on this computer" : projectName(path),
        granted: path !== UNASSIGNED && isGranted(path),
        items: items.sort((a, b) => b.createdAt.getTime() - a.createdAt.getTime()),
      }))
      .sort((a, b) => Number(a.path === UNASSIGNED) - Number(b.path === UNASSIGNED));
  }, [conversations, isGranted, projects, sessions]);

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
      <motion.h1
        initial={{ opacity: 0, y: 10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.4, ease: EASE_OUT, delay: 0.06 }}
        className="font-display mt-4 text-3xl font-semibold tracking-tight text-balance text-ink sm:text-4xl"
      >
        What should we build?
      </motion.h1>
      <p className="mt-2 max-w-md text-sm leading-6 text-ink-soft">
        Aloe reads, edits and runs code in the project you pick, with the same approvals as everything else on this computer.
      </p>

      <div className="mt-6 flex w-full max-w-md flex-wrap items-center justify-center gap-2 text-left">
        {projects.length > 0 && draftProject ? (
          <Select className="min-w-0 flex-1" value={draftProject} options={projectOptions} onChange={chooseProject} />
        ) : null}
        <button
          type="button"
          onClick={() => void addProject()}
          disabled={addingProject}
          className="press-tap inline-flex h-10 shrink-0 items-center gap-2 rounded-lg border border-edge bg-surface px-3 text-sm font-medium text-ink-soft transition-colors hover:bg-sage-soft hover:text-ink disabled:opacity-60"
        >
          {addingProject ? <Loader2 className="h-4 w-4 animate-spin text-moss" /> : <FolderPlus className="h-4 w-4" />}
          {projects.length ? "Add project" : "Choose a project folder"}
        </button>
      </div>
      {draftProject && branch ? (
        <p className="mt-2 inline-flex items-center gap-1.5 font-mono text-[11px] text-ink-soft">
          <GitBranch className="h-3 w-3" />
          {branch}
        </p>
      ) : null}
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
    <div className="flex items-center gap-3 border-b border-edge px-4 py-2 text-xs text-ink-soft sm:px-6">
      <span className="inline-flex min-w-0 items-center gap-1.5 font-medium text-ink" title={activeProject}>
        <Folder className="h-3.5 w-3.5 shrink-0 text-moss" />
        <span className="truncate">{projectName(activeProject)}</span>
      </span>
      {branch ? (
        <span className="inline-flex items-center gap-1 font-mono text-[11px]">
          <GitBranch className="h-3 w-3" />
          {branch}
        </span>
      ) : null}
      <span className="flex-1" />
      {pendingCount > 0 ? (
        <Link href="/app/desktop" className="inline-flex items-center gap-1.5 rounded-md bg-clay/10 px-2 py-1 font-semibold text-clay hover:bg-clay/15">
          <ShieldAlert className="h-3.5 w-3.5" />
          {pendingCount} command{pendingCount === 1 ? "" : "s"} waiting
        </Link>
      ) : (
        <Link href="/app/desktop" className="hidden hover:text-ink sm:inline">
          {TRUST_LABELS[config.commandTrustMode] ?? TRUST_LABELS.ask}
        </Link>
      )}
    </div>
  ) : null;

  return (
    <div className="app-wash flex h-full overflow-hidden text-ink">
      <aside className="liquid-glass-side hidden w-[264px] shrink-0 flex-col px-3 py-4 md:flex">
        <button
          type="button"
          onClick={() => openNewSession()}
          className="press-tap flex items-center gap-2 rounded-lg bg-sage-soft px-3 py-2 text-sm font-medium text-ink hover:bg-sage-soft/80"
        >
          <Plus className="h-4 w-4" />
          New session
        </button>

        <nav aria-label="Code sessions" className="hide-scrollbar mt-5 min-h-0 flex-1 space-y-5 overflow-y-auto">
          {groups.length === 0 ? (
            <p className="px-3 text-xs leading-5 text-ink-soft">Add a project folder and your sessions will be listed here, grouped by project.</p>
          ) : (
            groups.map((group) => (
              <section key={group.path}>
                <div className="group/project flex items-center gap-1 px-3">
                  <p className={cn("min-w-0 flex-1 truncate text-xs font-medium", group.granted || !group.path ? "text-ink-soft" : "text-ink-soft/50 line-through")} title={group.path || undefined}>
                    {group.label}
                  </p>
                  {group.granted ? (
                    <button
                      type="button"
                      onClick={() => openNewSession(group.path)}
                      aria-label={`New session in ${projectName(group.path)}`}
                      title={`New session in ${projectName(group.path)}`}
                      className="flex h-6 w-6 items-center justify-center rounded-md text-ink-soft opacity-0 transition-opacity hover:bg-sage-soft hover:text-ink focus-visible:opacity-100 group-hover/project:opacity-100"
                    >
                      <Plus className="h-3.5 w-3.5" />
                    </button>
                  ) : null}
                </div>
                <ul className="mt-1 space-y-0.5">
                  <AnimatePresence initial={false}>
                    {group.items.map((item) => (
                      <motion.li
                        key={item.id}
                        initial={{ opacity: 0, x: -8 }}
                        animate={{ opacity: 1, x: 0 }}
                        exit={{ opacity: 0, transition: { duration: 0.15 } }}
                        transition={{ duration: 0.2, ease: EASE_OUT }}
                        className="group/session relative"
                      >
                        <Link
                          href={`/app/code/${encodeURIComponent(item.id)}`}
                          className={cn(
                            "flex w-full flex-col rounded-lg py-1.5 pl-3 pr-8 text-left transition-colors",
                            item.id === sessionId ? "bg-sage-soft text-ink" : "text-ink-soft hover:bg-sage-soft/60 hover:text-ink",
                          )}
                        >
                          <span className="truncate text-sm">{item.title}</span>
                          <span className="font-mono text-[10px] uppercase tracking-wider text-ink-soft/60">{relativeTime(item.createdAt.toISOString())}</span>
                        </Link>
                        <button
                          type="button"
                          onClick={() => void removeSession(item.id)}
                          aria-label={`Delete ${item.title}`}
                          title="Delete session"
                          className="absolute right-1.5 top-1/2 flex h-6 w-6 -translate-y-1/2 items-center justify-center rounded-md text-ink-soft opacity-0 transition-opacity hover:bg-danger/10 hover:text-danger focus-visible:opacity-100 group-hover/session:opacity-100"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </button>
                      </motion.li>
                    ))}
                  </AnimatePresence>
                  {group.items.length === 0 ? <li className="px-3 py-1 text-xs text-ink-soft/60">No sessions yet</li> : null}
                </ul>
              </section>
            ))
          )}
        </nav>

        <button
          type="button"
          onClick={() => void addProject()}
          disabled={addingProject}
          className="press-tap mt-3 flex items-center gap-2 rounded-lg px-3 py-2 text-sm text-ink-soft hover:bg-sage-soft hover:text-ink disabled:opacity-60"
        >
          {addingProject ? <Loader2 className="h-4 w-4 animate-spin text-moss" /> : <FolderPlus className="h-4 w-4" />}
          Add project folder
        </button>
      </aside>

      <main className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <ChatPageClient
          routeBase="/app/code"
          workspace="code"
          messageContext={activeProject ? codeSessionContext(activeProject, branch) : null}
          emptyState={emptyState}
          header={header}
          onConversationId={handleConversationId}
          sendBlockedReason={sendBlockedReason}
        />
      </main>
    </div>
  );
}
