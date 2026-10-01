"use client";

import { useCallback, useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { FolderPlus, Loader2, X } from "lucide-react";

import { EASE_OUT } from "@aloe/ui/lib/motion";
import { cn } from "@aloe/ui/lib/utils";
import { getConversationFolders, revokeConversationFolder, shareConversationFolder, type ConversationFolder } from "../lib/agentApi";

type Props = {
    token: string | null;
    /** Null before the first message, when the conversation has no server row to attach a grant to. */
    conversationId: string | null;
    disabled?: boolean;
};

/** Trailing segment of a path, for a chip that has to fit next to the other composer controls. */
function folderName(folder: ConversationFolder): string {
    if (folder.label) return folder.label;
    const parts = folder.path.split(/[\\/]/).filter(Boolean);
    return parts[parts.length - 1] ?? folder.path;
}

/**
 * Shares a folder with this chat alone.
 *
 * Distinct from the folders granted in Aloe Desktop, which every conversation can reach forever.
 * A folder shared here works only in this chat and disappears from Aloe's reach the moment it is
 * removed — which is what makes it reasonable to point Aloe at a repo you would not hand it
 * permanently.
 *
 * The picking itself happens on the user's machine: pressing the button opens the OS folder dialog
 * in Aloe Desktop. That round trip is slow — a person is browsing a disk — so the button holds a
 * waiting state instead of pretending to be a normal fast action.
 */
export default function ConversationFolders({ token, conversationId, disabled = false }: Props) {
    const [folders, setFolders] = useState<ConversationFolder[]>([]);
    const [isPicking, setIsPicking] = useState(false);
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
        if (!token || !conversationId) {
            setFolders([]);
            return;
        }
        let cancelled = false;
        getConversationFolders(token, conversationId)
            .then((result) => {
                if (!cancelled) setFolders(result.folders);
            })
            // A failed read is not worth an error banner over the composer: the share button still
            // works, and pressing it reports anything genuinely wrong.
            .catch(() => undefined);
        return () => {
            cancelled = true;
        };
    }, [token, conversationId]);

    const share = useCallback(async () => {
        if (!token || !conversationId || isPicking) return;
        setIsPicking(true);
        setError(null);
        try {
            setFolders(await shareConversationFolder(token, conversationId));
        } catch (err) {
            setError(err instanceof Error ? err.message : "Could not share that folder.");
        } finally {
            setIsPicking(false);
        }
    }, [token, conversationId, isPicking]);

    const revoke = useCallback(
        async (grantId: string) => {
            if (!token || !conversationId) return;
            // Optimistic: taking access away should look instant, and a failed revoke is restored
            // by the reload below rather than leaving a chip that lies about being gone.
            const previous = folders;
            setFolders((current) => current.filter((folder) => folder.id !== grantId));
            try {
                setFolders(await revokeConversationFolder(token, conversationId, grantId));
            } catch (err) {
                setFolders(previous);
                setError(err instanceof Error ? err.message : "Could not remove that folder.");
            }
        },
        [token, conversationId, folders],
    );

    if (!token) return null;

    const shareDisabled = disabled || isPicking || !conversationId;

    return (
        <>
            <button
                type="button"
                onClick={() => void share()}
                disabled={shareDisabled}
                aria-label="Share a folder with this chat"
                title={
                    !conversationId
                        ? "Send a message first, then share a folder with this chat"
                        : isPicking
                          ? "Choose a folder in the Aloe Desktop window"
                          : "Share a folder with this chat only"
                }
                className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-ink-soft transition-colors hover:bg-sage-soft hover:text-ink disabled:cursor-not-allowed disabled:opacity-50"
            >
                {isPicking ? <Loader2 className="h-4 w-4 animate-spin text-moss" /> : <FolderPlus className="h-4 w-4" />}
            </button>

            <AnimatePresence initial={false}>
                {folders.map((folder) => (
                    <motion.span
                        key={folder.id}
                        initial={{ opacity: 0, transform: "translateY(-4px)" }}
                        animate={{ opacity: 1, transform: "translateY(0px)" }}
                        exit={{ opacity: 0 }}
                        transition={{ duration: 0.18, ease: EASE_OUT }}
                        title={folder.path}
                        className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg border border-edge bg-surface pl-2 pr-1 text-xs text-ink-soft"
                    >
                        <span className="max-w-28 truncate">{folderName(folder)}</span>
                        <button
                            type="button"
                            onClick={() => void revoke(folder.id)}
                            aria-label={`Stop sharing ${folderName(folder)} with this chat`}
                            className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-moss transition-colors hover:bg-sage-soft"
                        >
                            <X className="h-3 w-3" />
                        </button>
                    </motion.span>
                ))}
            </AnimatePresence>

            {isPicking && (
                <span className="shrink-0 text-[11px] text-ink-soft">Choose a folder in Aloe Desktop…</span>
            )}
            {error && (
                <span className={cn("shrink-0 text-[11px] text-clay")} title={error}>
                    <span className="max-w-48 truncate">{error}</span>
                </span>
            )}
        </>
    );
}
