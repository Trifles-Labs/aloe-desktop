import { useEffect, useRef } from "react";
import { motion, useReducedMotion } from "framer-motion";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { ArrowRight, Download, Loader2, X } from "lucide-react";

import { EASE_OUT } from "@aloe/ui/lib/motion";
import type { ReadyUpdate } from "../hooks/useAutoUpdate";

type Props = {
    update: ReadyUpdate;
    restarting: boolean;
    error: string | null;
    onRestart: () => void;
    onLater: () => void;
};

/* The release body is Markdown (see .github/workflows/release.yml). Headings become small section
   labels, the bold lead of a line reads as its title. Links show as text: this window has no way
   to open a browser, and following one would navigate Aloe itself away. */
const notesComponents: Components = {
    h1: ({ children }) => <p className="eyebrow mb-2 mt-5 first:mt-0">{children}</p>,
    h2: ({ children }) => <p className="eyebrow mb-2 mt-5 first:mt-0">{children}</p>,
    h3: ({ children }) => <p className="mb-1.5 mt-4 text-sm font-semibold text-ink first:mt-0">{children}</p>,
    p: ({ children }) => <p className="mb-3 text-sm leading-6 text-ink-soft last:mb-0">{children}</p>,
    strong: ({ children }) => <strong className="font-semibold text-ink">{children}</strong>,
    em: ({ children }) => <em className="italic">{children}</em>,
    ul: ({ children }) => <ul className="mb-3 space-y-1.5 last:mb-0">{children}</ul>,
    ol: ({ children }) => <ol className="mb-3 space-y-1.5 last:mb-0">{children}</ol>,
    li: ({ children }) => (
        <li className="flex gap-2.5 text-sm leading-6 text-ink-soft">
            <span aria-hidden className="mt-[0.6rem] h-1 w-1 shrink-0 rounded-full bg-moss" />
            <span className="min-w-0">{children}</span>
        </li>
    ),
    code: ({ children }) => <code className="rounded border border-edge px-1 font-mono text-[0.88em] text-ink">{children}</code>,
    a: ({ children }) => <span className="text-ink underline decoration-edge underline-offset-2">{children}</span>,
    img: () => null,
    hr: () => <hr className="my-4 border-edge" />,
};

/* The update is already downloaded when this opens, so it asks for one thing: a restart.
   It can appear mid-sentence, so focus lands on the dialog itself, not on Restart: a stray
   Enter must never close the app. Escape and the backdrop mean Later. */
export function UpdateModal({ update, restarting, error, onRestart, onLater }: Props) {
    const dialogRef = useRef<HTMLDivElement>(null);
    const reduceMotion = useReducedMotion();
    const notes = update.notes?.trim();

    useEffect(() => {
        dialogRef.current?.focus();
        const onKeyDown = (event: KeyboardEvent) => {
            if (event.key === "Escape" && !restarting) onLater();
        };
        window.addEventListener("keydown", onKeyDown);
        return () => window.removeEventListener("keydown", onKeyDown);
    }, [onLater, restarting]);

    return (
        <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
            className="fixed inset-0 z-[60] flex items-center justify-center bg-black/45 p-4 backdrop-blur-[3px]"
            onClick={(event) => {
                if (event.target === event.currentTarget && !restarting) onLater();
            }}
        >
            <motion.div
                ref={dialogRef}
                role="dialog"
                aria-modal="true"
                aria-labelledby="update-title"
                tabIndex={-1}
                initial={reduceMotion ? { opacity: 0 } : { opacity: 0, scale: 0.97, y: 8 }}
                animate={reduceMotion ? { opacity: 1 } : { opacity: 1, scale: 1, y: 0 }}
                exit={reduceMotion ? { opacity: 0 } : { opacity: 0, scale: 0.98, y: 4 }}
                transition={{ duration: 0.22, ease: EASE_OUT }}
                className="flex max-h-[min(36rem,calc(100vh-2rem))] w-full max-w-[26rem] flex-col overflow-hidden rounded-2xl border border-edge bg-surface-strong shadow-[0_24px_60px_rgba(0,0,0,0.35)] outline-none"
            >
                <div className="flex items-start gap-3.5 px-6 pb-5 pt-6">
                    <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-sage-soft text-moss">
                        <Download className="h-[18px] w-[18px]" />
                    </span>
                    <div className="min-w-0 flex-1">
                        <h2 id="update-title" className="font-display text-lg font-semibold leading-6 text-ink">
                            A new Aloe is ready
                        </h2>
                        <p className="mt-1 flex items-center gap-1.5 font-mono text-[11px] tabular-nums text-ink-soft">
                            <span>{update.currentVersion}</span>
                            <ArrowRight className="h-3 w-3 text-ink-soft/60" />
                            <span className="rounded-md bg-sage-soft px-1.5 py-0.5 font-semibold text-ink">{update.version}</span>
                        </p>
                    </div>
                    <button
                        type="button"
                        onClick={onLater}
                        disabled={restarting}
                        aria-label="Later"
                        className="-mr-2 -mt-1 inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-ink-soft transition-colors hover:bg-sage-soft hover:text-ink disabled:opacity-50"
                    >
                        <X className="h-4 w-4" />
                    </button>
                </div>

                {notes ? (
                    <div className="hide-scrollbar min-h-0 flex-1 overflow-y-auto border-t border-edge px-6 py-5">
                        <ReactMarkdown remarkPlugins={[remarkGfm]} components={notesComponents}>
                            {notes}
                        </ReactMarkdown>
                    </div>
                ) : null}

                <div className="border-t border-edge px-6 pb-5 pt-4">
                    {error ? <p className="mb-3 text-sm text-clay">{error}</p> : null}
                    <p className="mb-4 text-xs leading-5 text-ink-soft">
                        Already downloaded. Restarting takes a few seconds and stops anything Aloe is doing.
                    </p>
                    <div className="flex justify-end gap-2">
                        <button type="button" onClick={onLater} disabled={restarting} className="secondary-button px-4">
                            Later
                        </button>
                        <button type="button" onClick={onRestart} disabled={restarting} className="primary-button gap-2 px-4">
                            {restarting ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                            {restarting ? "Restarting…" : "Restart now"}
                        </button>
                    </div>
                </div>
            </motion.div>
        </motion.div>
    );
}
