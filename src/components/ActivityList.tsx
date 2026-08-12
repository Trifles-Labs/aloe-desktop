import React, { useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { Check, ChevronDown, Copy } from "lucide-react";

import Pill from "@/components/ui/Pill";
import { EASE_OUT } from "@/lib/motion";
import { relativeTime } from "@/lib/utils";
import type { RecentAction } from "../types";
import { formatTimestamp } from "../types";
import { copyText, humanizeKind } from "../lib/desktop";
import { ControlGroup, EmptyRow } from "./ControlGroup";

const statusTone = (status: string) => {
  const value = status.toLowerCase();
  if (["failed", "error", "denied"].includes(value)) return "danger" as const;
  if (["pending", "waiting", "running", "active", "in_progress"].includes(value)) return "gold" as const;
  return "moss" as const;
};

function Payload({ label, value }: { label: string; value: unknown }) {
  const [copied, setCopied] = useState(false);
  const json = JSON.stringify(value, null, 2);

  const copy = async () => {
    if (!(await copyText(json))) return;
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1600);
  };

  return (
    <div className="mt-3 first:mt-0">
      <div className="mb-1.5 flex items-center justify-between gap-2">
        <p className="spec-label">{label}</p>
        <button
          type="button"
          onClick={() => void copy()}
          className="press-tap inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] font-medium text-ink-soft hover:bg-sage-soft hover:text-ink"
        >
          {copied ? <Check className="h-3 w-3 text-moss" /> : <Copy className="h-3 w-3" />}
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      <pre className="max-h-56 overflow-auto rounded-lg border border-edge bg-surface p-3 font-mono text-[11px] leading-5 text-ink">{json}</pre>
    </div>
  );
}

function ActivityRow({ item }: { item: RecentAction }) {
  const [open, setOpen] = useState(false);
  const reduceMotion = useReducedMotion();

  return (
    /* The hairline lives on the wrapper: `.settings-row + .settings-row` can't
       see across it, and each row owns a collapsible panel below its header. */
    <div className="border-t border-edge first:border-t-0">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
        className="settings-row press-tap w-full text-left hover:bg-sage-soft/40"
      >
        <span className="min-w-0">
          <span className="block truncate text-[13px] font-medium leading-5 text-ink">{humanizeKind(item.kind)}</span>
          <span className="mt-0.5 block truncate text-xs leading-5 text-ink-soft">{item.detail || formatTimestamp(item.timestamp)}</span>
        </span>
        <span className="flex shrink-0 items-center gap-2.5">
          <span className="hidden text-[11px] text-ink-soft sm:inline" title={formatTimestamp(item.timestamp)}>
            {relativeTime(item.timestamp)}
          </span>
          <Pill tone={statusTone(item.status)} size="xs" className="uppercase tracking-[0.06em]">
            {item.status}
          </Pill>
          <ChevronDown className={`h-4 w-4 text-ink-soft transition-transform duration-150 ${open ? "rotate-180" : ""}`} />
        </span>
      </button>

      {/* Height is the one property with no transform equivalent here — the
          detail has to push the rows below it rather than cover them. */}
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            initial={reduceMotion ? { opacity: 0 } : { height: 0, opacity: 0 }}
            animate={reduceMotion ? { opacity: 1 } : { height: "auto", opacity: 1 }}
            exit={reduceMotion ? { opacity: 0 } : { height: 0, opacity: 0 }}
            transition={{ duration: 0.24, ease: EASE_OUT }}
            className="overflow-hidden"
          >
            <div className="border-t border-edge px-4 py-3.5">
              <dl className="grid gap-3 text-xs sm:grid-cols-2">
                <div>
                  <dt className="text-ink-soft">When</dt>
                  <dd className="mt-0.5 text-ink">{formatTimestamp(item.timestamp)}</dd>
                </div>
                <div className="min-w-0">
                  <dt className="text-ink-soft">Job ID</dt>
                  <dd className="mt-0.5 break-all font-mono text-[11px] text-ink">{item.jobId}</dd>
                </div>
              </dl>
              {item.input != null ? <Payload label="Input" value={item.input} /> : null}
              {item.output != null ? <Payload label="Output" value={item.output} /> : null}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

export function ActivityList({ actions }: { actions: RecentAction[] }) {
  const visible = actions.slice(-50).reverse();

  return (
    <ControlGroup label="Recent activity" className="overflow-hidden">
      {visible.length === 0 ? (
        <EmptyRow>Tool calls and local actions will appear here.</EmptyRow>
      ) : (
        visible.map((item) => <ActivityRow key={`${item.jobId}-${item.timestamp}`} item={item} />)
      )}
    </ControlGroup>
  );
}
