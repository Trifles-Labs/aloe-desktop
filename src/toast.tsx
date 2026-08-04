import React, { useCallback, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { CheckCircle2, Info, X, XCircle } from "lucide-react";
import type { Toast, ToastVariant } from "./types";

// ── Hook ──────────────────────────────────────────────────────────────────────

export function useToasts() {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const timers = useRef<Map<string, number>>(new Map());

  const dismiss = useCallback((id: string) => {
    window.clearTimeout(timers.current.get(id));
    timers.current.delete(id);
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const toast = useCallback(
    (message: string, variant: ToastVariant = "info") => {
      const id = Math.random().toString(36).slice(2, 10);
      setToasts((prev) => [...prev, { id, message, variant }]);
      const timer = window.setTimeout(() => dismiss(id), 4200);
      timers.current.set(id, timer);
    },
    [dismiss],
  );

  useEffect(() => {
    const ref = timers.current;
    return () => ref.forEach((t) => window.clearTimeout(t));
  }, []);

  return { toasts, toast, dismiss };
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
};

export function ToastContainer({ toasts, onDismiss }: ToastContainerProps) {
  if (toasts.length === 0) return null;
  return (
    <div className="pointer-events-none fixed right-6 top-6 z-50 flex w-full max-w-sm flex-col items-end gap-2" aria-live="polite">
      <AnimatePresence>
        {toasts.map((t) => (
          <motion.div
            key={t.id}
            initial={{ opacity: 0, x: 32, scale: 0.96 }}
            animate={{ opacity: 1, x: 0, scale: 1 }}
            exit={{ opacity: 0, x: 32, scale: 0.96 }}
            transition={{ duration: 0.25, ease: "easeOut" }}
            className={`pointer-events-auto flex w-full cursor-pointer items-start gap-2.5 rounded-2xl px-4 py-3 text-sm ${variantClass[t.variant]}`}
            onClick={() => onDismiss(t.id)}
            role="alert"
          >
            <ToastIcon variant={t.variant} />
            <span className="min-w-0 flex-1 leading-5">{t.message}</span>
            <button
              type="button"
              className="rounded-full p-1 transition-colors hover:bg-white/15"
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
