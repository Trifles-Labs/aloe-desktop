import React, { useCallback, useEffect, useRef, useState } from "react";
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

function ToastIcon({ variant }: { variant: ToastVariant }) {
  if (variant === "success") return <CheckCircle2 size={16} className="toast-icon" />;
  if (variant === "error")   return <XCircle      size={16} className="toast-icon" />;
  return                            <Info          size={16} className="toast-icon" />;
}

type ToastContainerProps = {
  toasts: Toast[];
  onDismiss: (id: string) => void;
};

export function ToastContainer({ toasts, onDismiss }: ToastContainerProps) {
  if (toasts.length === 0) return null;
  return (
    <div className="toast-container" aria-live="polite">
      {toasts.map((t) => (
        <div
          key={t.id}
          className={`toast toast-${t.variant}`}
          onClick={() => onDismiss(t.id)}
          role="alert"
        >
          <ToastIcon variant={t.variant} />
          <span className="toast-body">{t.message}</span>
          <button
            className="toast-close"
            onClick={(e) => { e.stopPropagation(); onDismiss(t.id); }}
            aria-label="Dismiss"
          >
            <X size={13} />
          </button>
        </div>
      ))}
    </div>
  );
}
