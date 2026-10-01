import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { AlertTriangle, Leaf, Loader2, ShieldCheck } from "lucide-react";

import { EASE_OUT } from "@aloe/ui/lib/motion";

type Props = {
  onGoogleSignIn: () => void;
  googleConnecting: boolean;
  error: string | null;
};

function GoogleIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} aria-hidden="true">
      <path
        fill="#4285F4"
        d="M23.49 12.27c0-.79-.07-1.54-.19-2.27H12v4.51h6.47c-.29 1.48-1.14 2.73-2.4 3.58v3h3.86c2.26-2.09 3.56-5.17 3.56-8.82z"
      />
      <path
        fill="#34A853"
        d="M12 24c3.24 0 5.95-1.08 7.93-2.91l-3.86-3c-1.08.72-2.45 1.16-4.07 1.16-3.13 0-5.78-2.11-6.73-4.96H1.29v3.09C3.26 21.3 7.31 24 12 24z"
      />
      <path
        fill="#FBBC05"
        d="M5.27 14.29c-.25-.72-.38-1.49-.38-2.29s.14-1.57.38-2.29V6.62H1.29C.47 8.24 0 10.06 0 12s.47 3.76 1.29 5.38l3.98-3.09z"
      />
      <path
        fill="#EA4335"
        d="M12 4.75c1.77 0 3.35.61 4.6 1.8l3.42-3.42C17.95 1.19 15.24 0 12 0 7.31 0 3.26 2.7 1.29 6.62l3.98 3.09C6.22 6.86 8.87 4.75 12 4.75z"
      />
    </svg>
  );
}

/* Pairing is sign-in, the way Claude Desktop does it: the browser runs Google,
   hands the session back over `aloe://`, and this device registers itself.
   There is no setup token to copy from the web app any more. */
export function AuthScreen({ onGoogleSignIn, googleConnecting, error }: Props) {
  const reduceMotion = useReducedMotion();

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
            <Leaf className="h-5 w-5" />
          </div>
          <div className="min-w-0">
            <p className="eyebrow">Get started</p>
            <p className="mt-0.5 text-lg font-semibold tracking-[-0.01em] text-ink">Sign in to Aloe Desktop</p>
          </div>
        </div>

        <p className="mt-4 text-[13px] leading-5 text-ink-soft">
          Your browser opens to finish signing in, then brings you back here. This computer is paired to your account automatically.
        </p>

        <button
          type="button"
          onClick={onGoogleSignIn}
          disabled={googleConnecting}
          className="primary-button press-tap mt-6 w-full disabled:cursor-not-allowed disabled:opacity-50"
        >
          {googleConnecting ? (
            <>
              <Loader2 className="h-4 w-4 animate-spin" />
              Waiting for your browser…
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
              className="overflow-hidden text-[13px] leading-5 text-danger"
            >
              <span className="mt-3 flex items-start gap-2">
                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                {error}
              </span>
            </motion.p>
          )}
        </AnimatePresence>

        <p className="mt-5 flex items-start gap-2 border-t border-edge pt-4 text-xs leading-5 text-ink-soft">
          <ShieldCheck className="mt-0.5 h-3.5 w-3.5 shrink-0 text-moss" />
          Aloe only reaches folders you grant it, and asks before running commands.
        </p>
      </motion.div>
    </div>
  );
}
