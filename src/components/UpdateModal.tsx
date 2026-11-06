import { useEffect, useRef } from "react";
import { motion, useReducedMotion } from "framer-motion";
import { Loader2, PartyPopper, X } from "lucide-react";

import { EASE_OUT } from "@aloe/ui/lib/motion";
import type { ReadyUpdate } from "../hooks/useAutoUpdate";

type Props = {
    update: ReadyUpdate;
    restarting: boolean;
    error: string | null;
    onRestart: () => void;
    onLater: () => void;
};

/* The update is already downloaded when this opens, so it asks for one thing: a restart.
   It can appear mid-sentence, so focus lands on the dialog itself, not on Restart: a stray
   Enter must never close the app. Escape and the backdrop mean Later. */
export function UpdateModal({ update, restarting, error, onRestart, onLater }: Props) {
    const dialogRef = useRef<HTMLDivElement>(null);
    const reduceMotion = useReducedMotion();

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
            className="fixed inset-0 z-[60] flex items-center justify-center bg-black/35 p-4 backdrop-blur-[2px]"
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
                initial={reduceMotion ? { opacity: 0 } : { opacity: 0, scale: 0.96, y: 12 }}
                animate={reduceMotion ? { opacity: 1 } : { opacity: 1, scale: 1, y: 0 }}
                exit={reduceMotion ? { opacity: 0 } : { opacity: 0, scale: 0.98, y: 6 }}
                transition={{ duration: 0.26, ease: EASE_OUT }}
                className="w-full max-w-md overflow-hidden rounded-2xl border border-edge bg-surface-strong shadow-[0_24px_60px_rgba(22,33,26,0.25)] outline-none"
            >
                <div className="relative border-b border-edge px-8 pb-6 pt-8">
                    <button
                        type="button"
                        onClick={onLater}
                        disabled={restarting}
                        aria-label="Later"
                        className="absolute right-5 top-5 inline-flex h-8 w-8 items-center justify-center rounded-md text-ink-soft transition-colors hover:bg-sage-soft hover:text-ink disabled:opacity-50"
                    >
                        <X className="h-4 w-4" />
                    </button>

                    <span className="inline-flex h-10 w-10 items-center justify-center rounded-xl bg-sage-soft text-moss">
                        <PartyPopper className="h-5 w-5" />
                    </span>
                    <p className="eyebrow mt-4">Update ready</p>
                    <h2 id="update-title" className="font-display mt-2 text-2xl font-semibold text-ink">
                        Aloe {update.version} is here
                    </h2>
                    <p className="mt-1 font-mono text-[11px] uppercase tracking-[0.12em] text-ink-soft/80">
                        You have {update.currentVersion}
                    </p>
                </div>

                <div className="px-8 pb-8 pt-6">
                    {update.notes ? (
                        <div className="mb-5 max-h-48 overflow-y-auto whitespace-pre-line rounded-xl border border-edge px-4 py-3 text-sm leading-6 text-ink">
                            {update.notes}
                        </div>
                    ) : null}
                    <p className="mb-6 text-sm leading-6 text-ink-soft">
                        It's already downloaded. Restarting takes a few seconds; anything Aloe is doing right now will stop.
                    </p>
                    {error ? <p className="mb-4 text-sm text-clay">{error}</p> : null}

                    <div className="flex gap-3">
                        <button type="button" onClick={onLater} disabled={restarting} className="secondary-button flex-1">
                            Later
                        </button>
                        <button type="button" onClick={onRestart} disabled={restarting} className="primary-button flex-1 gap-2">
                            {restarting ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                            {restarting ? "Restarting…" : "Restart now"}
                        </button>
                    </div>
                </div>
            </motion.div>
        </motion.div>
    );
}
