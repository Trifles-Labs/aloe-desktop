"use client";

import { useEffect, useRef } from "react";
import { motion } from "framer-motion";
import { ArrowUpRight, X } from "lucide-react";

import { CONNECTIONS_URL, openExternal } from "../lib/desktop";

type Props = {
    providerName: string;
    onDismiss: () => void;
};

export default function DesktopConnectModal({ providerName, onDismiss }: Props) {
    const overlayRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
        const handler = (e: KeyboardEvent) => { if (e.key === "Escape") onDismiss(); };
        window.addEventListener("keydown", handler);
        return () => window.removeEventListener("keydown", handler);
    }, [onDismiss]);

    return (
        <motion.div
            ref={overlayRef}
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            transition={{ duration: 0.2 }}
            className="fixed inset-0 z-50 flex items-center justify-center bg-black/35 p-4 backdrop-blur-[2px]"
            onClick={(e) => { if (e.target === overlayRef.current) onDismiss(); }}
        >
            <motion.div
                initial={{ opacity: 0, scale: 0.95, y: 12 }}
                animate={{ opacity: 1, scale: 1, y: 0 }}
                transition={{ duration: 0.25, ease: "easeOut" }}
                className="w-full max-w-md overflow-hidden rounded-2xl border border-edge bg-surface-strong shadow-[0_24px_60px_rgba(22,33,26,0.25)]"
            >
                <div className="relative border-b border-edge px-8 pb-6 pt-8">
                    <button
                        type="button"
                        onClick={onDismiss}
                        aria-label="Close"
                        className="absolute right-5 top-5 inline-flex h-8 w-8 items-center justify-center rounded-md text-ink-soft transition-colors hover:bg-sage-soft hover:text-ink"
                    >
                        <X className="h-4 w-4" />
                    </button>

                    <p className="eyebrow">Aloe Desktop</p>
                    <h2 className="font-display mt-2 text-2xl font-semibold text-ink">
                        Connect {providerName} from the browser
                    </h2>
                </div>

                <div className="px-8 pb-8 pt-6">
                    <p className="mb-6 text-sm leading-6 text-ink-soft">
                        Connecting {providerName}{" "}requires signing in through your browser. Open Aloe on the web to finish connecting, then come back &mdash; your account stays in sync everywhere.
                    </p>

                    <div className="flex gap-3">
                        <button type="button" onClick={onDismiss} className="secondary-button flex-1">
                            Maybe later
                        </button>
                        <button
                            type="button"
                            onClick={() => {
                                void openExternal(CONNECTIONS_URL);
                                onDismiss();
                            }}
                            className="primary-button flex-1 gap-2"
                        >
                            Open Aloe in browser
                            <ArrowUpRight className="h-4 w-4" />
                        </button>
                    </div>
                </div>
            </motion.div>
        </motion.div>
    );
}
