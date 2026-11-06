import { useEffect, useRef } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { AlertTriangle, Loader2 } from "lucide-react";

import { EASE_OUT } from "@aloe/ui/lib/motion";

type Props = {
  onGoogleSignIn: () => void;
  googleConnecting: boolean;
  error: string | null;
};

function GoogleIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} aria-hidden="true">
      <path fill="#4285F4" d="M23.49 12.27c0-.79-.07-1.54-.19-2.27H12v4.51h6.47c-.29 1.48-1.14 2.73-2.4 3.58v3h3.86c2.26-2.09 3.56-5.17 3.56-8.82z" />
      <path fill="#34A853" d="M12 24c3.24 0 5.95-1.08 7.93-2.91l-3.86-3c-1.08.72-2.45 1.16-4.07 1.16-3.13 0-5.78-2.11-6.73-4.96H1.29v3.09C3.26 21.3 7.31 24 12 24z" />
      <path fill="#FBBC05" d="M5.27 14.29c-.25-.72-.38-1.49-.38-2.29s.14-1.57.38-2.29V6.62H1.29C.47 8.24 0 10.06 0 12s.47 3.76 1.29 5.38l3.98-3.09z" />
      <path fill="#EA4335" d="M12 4.75c1.77 0 3.35.61 4.6 1.8l3.42-3.42C17.95 1.19 15.24 0 12 0 7.31 0 3.26 2.7 1.29 6.62l3.98 3.09C6.22 6.86 8.87 4.75 12 4.75z" />
    </svg>
  );
}

function FitWord({ children }: { children: string }) {
  const wrap = useRef<HTMLDivElement>(null);
  const text = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    const w = wrap.current;
    const t = text.current;
    if (!w || !t) return;
    const fit = () => {
      t.style.fontSize = "100px";
      const ratio = w.clientWidth / Math.max(1, t.scrollWidth);
      t.style.fontSize = `${Math.max(1, Math.floor(100 * ratio * 100) / 100)}px`;
    };
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(w);
    void document.fonts?.ready.then(fit);
    return () => ro.disconnect();
  }, [children]);

  return (
    <div ref={wrap} className="w-full overflow-hidden" aria-hidden>
      <span
        ref={text}
        className="aloe-wordmark inline-block whitespace-nowrap uppercase text-ink"
        style={{ fontWeight: 900, lineHeight: 0.78, letterSpacing: "-0.045em" }}
      >
        {children}
      </span>
    </div>
  );
}

export function AuthScreen({ onGoogleSignIn, googleConnecting, error }: Props) {
  const reduceMotion = useReducedMotion();

  return (
    <div className="flex h-full min-h-0 w-full flex-col">
      <div className="flex items-center justify-between px-8 pt-6 sm:px-10">
        <p className="eyebrow">01 — Desktop</p>
        <p className="eyebrow">Sign in</p>
      </div>

      <motion.div
        initial={reduceMotion ? { opacity: 0 } : { opacity: 0, transform: "translateY(10px)" }}
        animate={{ opacity: 1, transform: "translateY(0px)" }}
        transition={{ duration: 0.45, ease: EASE_OUT }}
        className="grid items-end gap-8 px-8 pt-10 sm:px-10 lg:grid-cols-12 lg:gap-10 lg:pt-14"
      >
        <h1 className="aloe-serif text-[clamp(3.2rem,6.4vw,6.25rem)] text-ink lg:col-span-8">
          Talk to your computer.
          <em className="mt-1 block">It gets it done.</em>
        </h1>

        <div className="max-w-sm lg:col-span-4 lg:pb-2">
          <p className="text-[1rem] leading-[1.6] text-ink-soft">
            Your browser opens to finish signing in, then brings you back here.
          </p>
          <button
            type="button"
            onClick={onGoogleSignIn}
            disabled={googleConnecting}
            className="mt-6 inline-flex h-[3.25rem] items-center gap-3 rounded-full bg-[#c8f36a] px-6 text-[0.95rem] font-semibold text-[#0b0d0c] transition-transform hover:scale-[1.03] disabled:cursor-not-allowed disabled:opacity-50"
          >
            {googleConnecting ? (
              <>
                <Loader2 className="h-4 w-4 animate-spin" />
                Waiting for your browser
              </>
            ) : (
              <>
                <GoogleIcon className="h-4 w-4" />
                Continue with Google
              </>
            )}
          </button>

          <AnimatePresence initial={false}>
            {error && (
              <motion.p
                initial={reduceMotion ? { opacity: 0 } : { opacity: 0, height: 0 }}
                animate={{ opacity: 1, height: "auto" }}
                exit={{ opacity: 0, height: 0 }}
                transition={{ duration: 0.2, ease: EASE_OUT }}
                role="alert"
                className="mt-4 flex items-start gap-2 overflow-hidden text-[13px] leading-5 text-danger"
              >
                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                {error}
              </motion.p>
            )}
          </AnimatePresence>
        </div>
      </motion.div>

      <div className="min-h-8 flex-1" />

      <div className="px-2 pb-1">
        <FitWord>ALOE</FitWord>
      </div>
    </div>
  );
}
