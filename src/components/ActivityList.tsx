import React, { useState } from "react";
import { ChevronDown, History } from "lucide-react";
import type { RecentAction } from "../types";
import { formatTimestamp } from "../types";

const statusTone = (status: string) => {
  const value = status.toLowerCase();
  if (["failed", "error", "denied"].includes(value)) return "bg-clay/12 text-danger";
  if (["pending", "waiting", "running", "active", "in_progress"].includes(value)) return "bg-gold/15 text-gold";
  return "bg-sage text-moss";
};

function ActivityRow({ item }: { item: RecentAction }) {
  const [open, setOpen] = useState(false);
  return (
    <article className="border-b border-edge last:border-0">
      <button className="flex w-full items-center gap-3 py-4 text-left" onClick={() => setOpen((value) => !value)} aria-expanded={open}>
        <div className="min-w-0 flex-1"><p className="truncate text-sm font-semibold text-ink">{item.kind.replaceAll("_", " ")}</p><p className="mt-0.5 truncate text-xs text-ink-soft">{item.detail || formatTimestamp(item.timestamp)}</p></div>
        <span className={`rounded-full px-2.5 py-1 text-[10px] font-semibold uppercase ${statusTone(item.status)}`}>{item.status}</span>
        <ChevronDown className={`h-4 w-4 text-moss transition-transform ${open ? "rotate-180" : ""}`} />
      </button>
      {open ? <div className="mb-4 rounded-2xl bg-sage-soft p-4 text-xs"><dl className="grid gap-3 sm:grid-cols-2"><div><dt className="text-ink-soft">When</dt><dd className="mt-1 text-ink">{formatTimestamp(item.timestamp)}</dd></div><div><dt className="text-ink-soft">Job ID</dt><dd className="mt-1 break-all font-mono text-ink">{item.jobId}</dd></div></dl>{item.input != null ? <div className="mt-4"><p className="mb-2 text-[10px] font-bold uppercase tracking-[0.16em] text-ink-soft">Input</p><pre className="max-h-64 overflow-auto rounded-xl border border-edge bg-surface-strong p-4 font-mono text-xs leading-5 text-ink">{JSON.stringify(item.input, null, 2)}</pre></div> : null}{item.output != null ? <div className="mt-4"><p className="mb-2 text-[10px] font-bold uppercase tracking-[0.16em] text-ink-soft">Output</p><pre className="max-h-64 overflow-auto rounded-xl border border-edge bg-surface-strong p-4 font-mono text-xs leading-5 text-ink">{JSON.stringify(item.output, null, 2)}</pre></div> : null}</div> : null}
    </article>
  );
}

export function ActivityList({ actions }: { actions: RecentAction[] }) {
  const visible = actions.slice(-50).reverse();
  return (
    <section className="liquid-glass rounded-3xl p-5 sm:p-6">
      <div className="flex items-center justify-between"><div><p className="eyebrow">History</p><h2 className="mt-1 font-display text-xl font-semibold text-ink">Recent activity</h2></div><History className="h-5 w-5 text-moss" /></div>
      <div className="mt-4">{visible.length === 0 ? <p className="rounded-2xl border border-dashed border-edge px-4 py-6 text-center text-sm text-ink-soft">Tool calls and local actions will appear here.</p> : visible.map((item) => <ActivityRow key={`${item.jobId}-${item.timestamp}`} item={item} />)}</div>
    </section>
  );
}
