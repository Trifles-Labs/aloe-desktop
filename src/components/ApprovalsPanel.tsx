import React from "react";
import { CheckCircle2, Play, RefreshCw, ShieldCheck, X } from "lucide-react";
import type { AgentConfig, PendingApproval } from "../types";

type Props = {
  config: AgentConfig;
  pending: PendingApproval[];
  onRefresh: () => void;
  onApprove: (jobId: string, approved: boolean) => void;
  onToggleAlwaysAllow: (enabled: boolean) => void;
};

export function ApprovalsPanel({ config, pending, onRefresh, onApprove, onToggleAlwaysAllow }: Props) {
  return (
    <section className="liquid-glass rounded-3xl p-5 sm:p-6">
      <div className="flex items-center justify-between gap-4">
        <div><p className="eyebrow">Safety</p><h2 className="mt-1 font-display text-xl font-semibold text-[#0b3026] dark:text-[#e8f0e0]">Command approvals</h2></div>
        <button className="secondary-button min-h-0 px-3 py-2 text-xs" onClick={onRefresh}><RefreshCw className="h-3.5 w-3.5" />Refresh</button>
      </div>

      <label className="mt-5 flex cursor-pointer items-start gap-3 rounded-2xl border border-[#d9e0d5] bg-white/40 p-4 dark:border-[#2a3a28] dark:bg-[#152118]/55">
        <input className="mt-0.5 h-4 w-4 accent-[#6f8747]" type="checkbox" checked={config.commandTrustMode === "trusted_coding"} onChange={(event) => onToggleAlwaysAllow(event.target.checked)} />
        <span><strong className="block text-sm text-[#0b3026] dark:text-[#e8f0e0]">Trusted Coding mode</strong><small className="mt-1 block text-xs leading-5 text-[#6b786f] dark:text-[#78907e]">Run recognized inspection, install, build, and test commands automatically. Dangerous commands still require approval. Commands use your OS permissions; this is not a complete sandbox.</small></span>
      </label>

      <div className="mt-4 space-y-3">
        {pending.length === 0 ? (
          <div className="rounded-2xl border border-dashed border-[#d9e0d5] p-6 text-center dark:border-[#2a3a28]">
            {config.commandTrustMode === "trusted_coding" ? <ShieldCheck className="mx-auto h-7 w-7 text-[#6f8747] dark:text-[#8faa5f]" /> : <CheckCircle2 className="mx-auto h-7 w-7 text-[#6f8747] dark:text-[#8faa90]" />}
            <p className="mt-2 text-sm font-medium text-[#506257] dark:text-[#8aaa90]">{config.commandTrustMode === "trusted_coding" ? "Recognized coding commands can run automatically." : "Nothing waiting for approval."}</p>
          </div>
        ) : null}
        {pending.map((item) => (
          <article className="rounded-2xl border border-[#d9e0d5] bg-white/45 p-4 dark:border-[#2a3a28] dark:bg-[#152118]/55" key={item.jobId}>
            <p className="text-sm font-semibold text-[#0b3026] dark:text-[#e8f0e0]">{item.reason}</p>
            <code className="mt-3 block overflow-x-auto rounded-xl bg-[#edf2e8] px-3 py-2 text-xs text-[#0b3026] dark:bg-[#0e1a13] dark:text-[#cbd9c6]">{item.command}</code>
            <p className="mt-2 truncate text-xs text-[#6b786f] dark:text-[#78907e]">{item.cwd}</p>
            <div className="mt-4 flex gap-2"><button className="primary-button min-h-0 px-4 py-2 text-xs" onClick={() => onApprove(item.jobId, true)}><Play className="h-3.5 w-3.5" />Run</button><button className="secondary-button min-h-0 px-4 py-2 text-xs" onClick={() => onApprove(item.jobId, false)}><X className="h-3.5 w-3.5" />Deny</button></div>
          </article>
        ))}
      </div>
    </section>
  );
}
