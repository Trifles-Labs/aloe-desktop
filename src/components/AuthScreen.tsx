import React, { useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { AlertTriangle, ArrowRight, ClipboardPaste, ExternalLink, Leaf, Loader2, ShieldCheck } from "lucide-react";

import { EASE_OUT } from "@/lib/motion";
import { CONNECTIONS_URL, openExternal, readClipboard } from "../lib/desktop";

type Props = {
  setupToken: string;
  onTokenChange: (value: string) => void;
  onConnect: () => void;
  connecting: boolean;
  error: string | null;
};

/* First run is the one screen every user sees, and the old one asked for a
   token without saying where a token comes from. It is three steps now, and
   the first of them is a button that opens the page holding the token. */
const STEPS = [
  "Open Aloe in your browser and go to Settings → Connections.",
  "Copy the desktop setup token.",
  "Paste it below to pair this computer.",
];

export function AuthScreen({ setupToken, onTokenChange, onConnect, connecting, error }: Props) {
  const reduceMotion = useReducedMotion();
  const [pasteFailed, setPasteFailed] = useState(false);
  const fieldRef = useRef<HTMLTextAreaElement>(null);
  const ready = Boolean(setupToken.trim()) && !connecting;

  const paste = async () => {
    const text = await readClipboard();
    if (text?.trim()) {
      onTokenChange(text.trim());
      setPasteFailed(false);
      fieldRef.current?.focus();
    } else {
      setPasteFailed(true);
    }
  };

  /* The token is a single line, so Enter is the commit key here rather than a
     newline — the field is a textarea only because the string is long. */
  const onKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      if (ready) onConnect();
    }
  };

  return (
    <div className="flex min-h-full items-center justify-center px-6 py-10">
      <motion.div
        initial={reduceMotion ? { opacity: 0 } : { opacity: 0, transform: "translateY(10px)" }}
        animate={{ opacity: 1, transform: "translateY(0px)" }}
        transition={{ duration: 0.28, ease: EASE_OUT }}
        className="liquid-glass w-full max-w-md p-6 sm:p-7"
      >
        <div className="flex items-center gap-3">
          <div className="brand-mark h-10 w-10">
            <Leaf className="h-4.5 w-4.5" />
          </div>
          <div className="min-w-0">
            <p className="eyebrow">Get started</p>
            <p className="mt-0.5 text-lg font-semibold tracking-[-0.01em] text-ink">Connect this computer</p>
          </div>
        </div>

        <ol className="mt-6 space-y-3">
          {STEPS.map((step, index) => (
            <li key={step} className="flex gap-3">
              <span className="mt-px flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-sage-soft font-mono text-[11px] font-semibold text-moss">
                {index + 1}
              </span>
              <span className="text-[13px] leading-5 text-ink-soft">{step}</span>
            </li>
          ))}
        </ol>

        <button type="button" onClick={() => void openExternal(CONNECTIONS_URL)} className="secondary-button press-tap mt-4 w-full">
          Open Aloe Connections
          <ExternalLink className="h-4 w-4" />
        </button>

        <label className="mt-6 block">
          <span className="spec-label">Setup token</span>
          <div className="relative mt-2">
            <textarea
              ref={fieldRef}
              value={setupToken}
              onChange={(event) => onTokenChange(event.target.value)}
              onKeyDown={onKeyDown}
              placeholder="aloe_setup_…"
              rows={3}
              spellCheck={false}
              autoComplete="off"
              disabled={connecting}
              className="w-full resize-none rounded-xl border border-edge bg-surface px-3.5 py-3 pr-11 font-mono text-[13px] leading-5 text-ink outline-none transition-[border-color,box-shadow] duration-150 placeholder:font-sans placeholder:text-ink-soft/60 focus:border-moss focus:ring-2 focus:ring-moss/25 disabled:opacity-60"
            />
            <button
              type="button"
              onClick={() => void paste()}
              title="Paste from clipboard"
              aria-label="Paste from clipboard"
              className="press-tap absolute right-2 top-2 inline-flex h-8 w-8 items-center justify-center rounded-lg text-ink-soft hover:bg-sage-soft hover:text-ink"
            >
              <ClipboardPaste className="h-4 w-4" />
            </button>
          </div>
        </label>

        {/* Failures arrive here, next to the field that caused them, instead of
            in a toast that has already gone by the time you look up. */}
        <AnimatePresence initial={false}>
          {(error || pasteFailed) && (
            <motion.p
              initial={reduceMotion ? { opacity: 0 } : { opacity: 0, height: 0 }}
              animate={{ opacity: 1, height: "auto" }}
              exit={{ opacity: 0, height: 0 }}
              transition={{ duration: 0.2, ease: EASE_OUT }}
              role="alert"
              className="overflow-hidden text-[13px] leading-5 text-danger"
            >
              <span className="mt-3 flex items-start gap-2">
                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                {error ?? "Nothing to paste — copy the token first."}
              </span>
            </motion.p>
          )}
        </AnimatePresence>

        <button type="button" onClick={onConnect} disabled={!ready} className="primary-button press-tap mt-5 w-full disabled:cursor-not-allowed disabled:opacity-50">
          {connecting ? (
            <>
              <Loader2 className="h-4 w-4 animate-spin" />
              Connecting…
            </>
          ) : (
            <>
              Pair this device
              <ArrowRight className="h-4 w-4" />
            </>
          )}
        </button>

        <p className="mt-5 flex items-start gap-2 border-t border-edge pt-4 text-xs leading-5 text-ink-soft">
          <ShieldCheck className="mt-0.5 h-3.5 w-3.5 shrink-0 text-moss" />
          The token is exchanged once and stays on this device. Aloe only reaches folders you grant it.
        </p>
      </motion.div>
    </div>
  );
}
