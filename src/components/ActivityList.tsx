import React, { useState } from "react";
import { ChevronDown, History } from "lucide-react";
import type { RecentAction } from "../types";
import { formatTimestamp } from "../types";

const statusTone = (status: string) => {
  const value = status.toLowerCase();
  if (["failed", "error", "denied"].includes(value)) return "bg-[#fff1ed] text-[#a65345] dark:bg-[#33211d] dark:text-[#e8917f]";
  if (["pending", "waiting", "running", "active", "in_progress"].includes(value)) return "bg-[#fff8db] text-[#8a6a18] dark:bg-[#302a16] dark:text-[#d8bd68]";
  return "bg-[#edf4e7] text-[#4d6c35] dark:bg-[#213225] dark:text-[#9fbd7a]";
};

function ActivityRow({ item }: { item: RecentAction }) {
  const [open, setOpen] = useState(false);
  return (
    <article className="border-b border-[#dfe5da] last:border-0 dark:border-[#2a3a28]">
      <button className="flex w-full items-center gap-3 py-4 text-left" onClick={() => setOpen((value) => !value)} aria-expanded={open}>
        <div className="min-w-0 flex-1"><p className="truncate text-sm font-semibold text-[#0b3026] dark:text-[#e8f0e0]">{item.kind.replaceAll("_", " ")}</p><p className="mt-0.5 truncate text-xs text-[#6b786f] dark:text-[#78907e]">{item.detail || formatTimestamp(item.timestamp)}</p></div>
        <span className={`rounded-full px-2.5 py-1 text-[10px] font-semibold uppercase ${statusTone(item.status)}`}>{item.status}</span>
        <ChevronDown className={`h-4 w-4 text-[#6f8747] transition-transform ${open ? "rotate-180" : ""}`} />
      </button>
      {open ? <div className="mb-4 rounded-2xl bg-white/45 p-4 text-xs dark:bg-[#152118]/55"><dl className="grid gap-3 sm:grid-cols-2"><div><dt className="text-[#6b786f] dark:text-[#91a997]">When</dt><dd className="mt-1 text-[#0b3026] dark:text-[#f0f5eb]">{formatTimestamp(item.timestamp)}</dd></div><div><dt className="text-[#6b786f] dark:text-[#91a997]">Job ID</dt><dd className="mt-1 break-all font-mono text-[#0b3026] dark:text-[#f0f5eb]">{item.jobId}</dd></div></dl>{item.input != null ? <div className="mt-4"><p className="mb-2 text-[10px] font-bold uppercase tracking-[0.16em] text-[#506257] dark:text-[#a9bea9]">Input</p><pre className="max-h-64 overflow-auto rounded-xl border border-[#d9e0d5] bg-[#f5f8f1] p-4 font-mono text-xs leading-5 text-[#173f31] dark:border-[#344736] dark:bg-[#09150e] dark:text-[#c7ddc3]">{JSON.stringify(item.input, null, 2)}</pre></div> : null}{item.output != null ? <div className="mt-4"><p className="mb-2 text-[10px] font-bold uppercase tracking-[0.16em] text-[#506257] dark:text-[#a9bea9]">Output</p><pre className="max-h-64 overflow-auto rounded-xl border border-[#d9e0d5] bg-[#f5f8f1] p-4 font-mono text-xs leading-5 text-[#173f31] dark:border-[#344736] dark:bg-[#09150e] dark:text-[#c7ddc3]">{JSON.stringify(item.output, null, 2)}</pre></div> : null}</div> : null}
    </article>
  );
}

export function ActivityList({ actions }: { actions: RecentAction[] }) {
  const visible = actions.slice(-50).reverse();
  return (
    <section className="liquid-glass rounded-3xl p-5 sm:p-6">
      <div className="flex items-center justify-between"><div><p className="eyebrow">History</p><h2 className="mt-1 font-display text-xl font-semibold text-[#0b3026] dark:text-[#e8f0e0]">Recent activity</h2></div><History className="h-5 w-5 text-[#6f8747] dark:text-[#8faa5f]" /></div>
      <div className="mt-4">{visible.length === 0 ? <p className="rounded-2xl border border-dashed border-[#d9e0d5] px-4 py-6 text-center text-sm text-[#6b786f] dark:border-[#2a3a28] dark:text-[#78907e]">Tool calls and local actions will appear here.</p> : visible.map((item) => <ActivityRow key={`${item.jobId}-${item.timestamp}`} item={item} />)}</div>
    </section>
  );
}
