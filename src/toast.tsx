import React, { useCallback, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { CheckCircle2, Info, X, XCircle } from "lucide-react";

import { EASE_OUT } from "@/lib/motion";
import type { Toast, ToastVariant } from "./types";

const LIFETIME = 4200;

// ── Hook ──────────────────────────────────────────────────────────────────────

export function useToasts() {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const timers = useRef<Map<string, number>>(new Map());
  /* Remaining lifetime per toast, so hovering can hold one open and let go
     where it left off — a message that expires while you are reading it is the
     one failure mode of a timed toast. */
  const remaining = useRef<Map<string, { left: number; startedAt: number }>>(new Map());

  const dismiss = useCallback((id: string) => {
    window.clearTimeout(timers.current.get(id));
    timers.current.delete(id);
    remaining.current.delete(id);
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const schedule = useCallback(
    (id: string, left: number) => {
      window.clearTimeout(timers.current.get(id));
      remaining.current.set(id, { left, startedAt: Date.now() });
      timers.current.set(id, window.setTimeout(() => dismiss(id), left));
    },
    [dismiss],
  );

  const toast = useCallback(
    (message: string, variant: ToastVariant = "info") => {
      const id = Math.random().toString(36).slice(2, 10);
      // Three at a time is already more than anyone reads; the oldest goes.
      setToasts((prev) => [...prev, { id, message, variant }].slice(-3));
      schedule(id, LIFETIME);
    },
    [schedule],
  );

  const pause = useCallback((id: string) => {
    const entry = remaining.current.get(id);
    if (!entry) return;
    window.clearTimeout(timers.current.get(id));
    remaining.current.set(id, { left: Math.max(600, entry.left - (Date.now() - entry.startedAt)), startedAt: Date.now() });
  }, []);

  const resume = useCallback(
    (id: string) => {
      const entry = remaining.current.get(id);
      if (entry) schedule(id, entry.left);
    },
    [schedule],
  );

  useEffect(() => {
    const ref = timers.current;
    return () => ref.forEach((t) => window.clearTimeout(t));
  }, []);

  return { toasts, toast, dismiss, pause, resume };
}

// ── Container ─────────────────────────────────────────────────────────────────

// Fixed (not theme-token) colors on purpose, matching the web app's toast — semantic tokens
// like `ink`/`danger` invert between light/dark mode for use as text-on-surface, which washes
// out white-on-white when used as a solid fill instead. Toasts should look the same, and stay
// legible, in both themes.
const variantClass: Record<ToastVariant, string> = {
  success: "bg-[#2f6b4f] text-white shadow-[0_12px_32px_rgba(47,107,79,0.3)]",
  error: "bg-[#b23a20] text-white shadow-[0_12px_32px_rgba(178,58,32,0.3)]",
  info: "bg-[#1f4a37] text-[#f4f7f0] shadow-[0_12px_32px_rgba(31,74,55,0.3)]",
};

function ToastIcon({ variant }: { variant: ToastVariant }) {
  if (variant === "success") return <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0" />;
  if (variant === "error") return <XCircle className="mt-0.5 h-4 w-4 shrink-0" />;
  return <Info className="mt-0.5 h-4 w-4 shrink-0" />;
}

type ToastContainerProps = {
  toasts: Toast[];
  onDismiss: (id: string) => void;
  onPause?: (id: string) => void;
  onResume?: (id: string) => void;
};

export function ToastContainer({ toasts, onDismiss, onPause, onResume }: ToastContainerProps) {
  return (
    /* Below the title bar rather than under it — the bar is z-100 chrome, and a
       toast that slides beneath the window controls looks like a bug. */
    <div className="pointer-events-none fixed right-5 top-[3.25rem] z-50 flex w-full max-w-sm flex-col items-end gap-2" aria-live="polite">
      <AnimatePresence initial={false}>
        {toasts.map((t) => (
          <motion.div
            key={t.id}
            layout
            initial={{ opacity: 0, transform: "translateX(28px) scale(0.96)" }}
            animate={{ opacity: 1, transform: "translateX(0px) scale(1)" }}
            /* Leaves the way it came in — the same edge, the same path. */
            exit={{ opacity: 0, transform: "translateX(28px) scale(0.96)" }}
            transition={{ duration: 0.24, ease: EASE_OUT }}
            className={`pointer-events-auto flex w-full cursor-pointer items-start gap-2.5 rounded-2xl px-4 py-3 text-sm ${variantClass[t.variant]}`}
            onClick={() => onDismiss(t.id)}
            onMouseEnter={() => onPause?.(t.id)}
            onMouseLeave={() => onResume?.(t.id)}
            onFocus={() => onPause?.(t.id)}
            onBlur={() => onResume?.(t.id)}
            role="alert"
          >
            <ToastIcon variant={t.variant} />
            <span className="min-w-0 flex-1 leading-5">{t.message}</span>
            <button
              type="button"
              className="rounded-full p-1 transition-colors duration-100 hover:bg-white/15"
              onClick={(e) => { e.stopPropagation(); onDismiss(t.id); }}
              aria-label="Dismiss"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  );
}
