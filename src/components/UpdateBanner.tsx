import React, { useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { PartyPopper, X } from "lucide-react";

import { EASE_OUT } from "@/lib/motion";

/* A new build is already downloaded by the time this appears, so the honest
   framing is "next launch, or now" — not a demand. It opens by growing into
   place rather than appearing mid-page, because it pushes the app down. */

export function UpdateBanner({ onRestart }: { onRestart: () => void }) {
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
              A new version of Aloe is ready. It installs the next time you launch — or restart now.
            </span>
            <button type="button" onClick={onRestart} className="press-tap rounded-lg bg-pine px-3 py-1.5 text-xs font-semibold text-cream hover:bg-pine-hover">
              Restart now
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
