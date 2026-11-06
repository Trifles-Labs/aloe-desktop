import React, { useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { Loader2, PartyPopper, X } from "lucide-react";

import { EASE_OUT } from "@aloe/ui/lib/motion";

/* The quiet reminder left behind after "Later" on the update modal. It opens by growing into
   place rather than appearing mid-page, because it pushes the app down. Nothing installs on its
   own: the update waits for a restart from here or the modal. */

export function UpdateBanner({ version, restarting, onRestart }: { version: string; restarting: boolean; onRestart: () => void }) {
  const [dismissed, setDismissed] = useState(false);
  const reduceMotion = useReducedMotion();

  return (
    <AnimatePresence initial={false}>
      {!dismissed && (
        <motion.div
          role="status"
          initial={reduceMotion ? { opacity: 0 } : { height: 0, opacity: 0 }}
          animate={reduceMotion ? { opacity: 1 } : { height: "auto", opacity: 1 }}
          exit={reduceMotion ? { opacity: 0 } : { height: 0, opacity: 0 }}
          transition={{ duration: 0.28, ease: EASE_OUT }}
          className="shrink-0 overflow-hidden border-b border-edge bg-sage-soft"
        >
          <div className="flex items-center gap-3 px-4 py-2 text-[13px] text-ink">
            <PartyPopper className="h-4 w-4 shrink-0 text-moss" />
            <span className="min-w-0 flex-1">
              Aloe {version} is ready. Restart to update.
            </span>
            <button type="button" onClick={onRestart} disabled={restarting} className="press-tap inline-flex items-center gap-1.5 rounded-lg bg-pine px-3 py-1.5 text-xs font-semibold text-cream hover:bg-pine-hover disabled:opacity-60">
              {restarting ? <Loader2 className="h-3 w-3 animate-spin" /> : null}
              {restarting ? "Restarting…" : "Restart now"}
            </button>
            <button
              type="button"
              onClick={() => setDismissed(true)}
              aria-label="Dismiss"
              title="Later"
              className="press-tap inline-flex h-7 w-7 items-center justify-center rounded-lg text-ink-soft hover:bg-sage hover:text-ink"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
