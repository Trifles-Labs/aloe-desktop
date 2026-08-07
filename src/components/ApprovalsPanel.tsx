import React from "react";
import Link from "next/link";
import { ArrowUpRight, CheckCircle2, RefreshCw, ShieldAlert, ShieldCheck } from "lucide-react";
import type { AgentConfig, CommandTrustMode, PendingApproval } from "../types";

type Props = {
  config: AgentConfig;
  pending: PendingApproval[];
  onRefresh: () => void;
  onSetCommandTrustMode: (mode: CommandTrustMode) => void;
};

const commandModes: Array<{ mode: CommandTrustMode; title: string; description: string }> = [
  {
    mode: "ask",
    title: "Ask before commands",
    description: "Every shell command waits here for review before it runs.",
  },
  {
    mode: "trusted_coding",
    title: "Trusted Coding mode",
    description: "Run recognized inspection, install, build, and test commands automatically. Dangerous commands still require approval.",
  },
  {
    mode: "all",
    title: "Allow all commands",
    description: "Run every command automatically with your OS permissions. Use only when you fully trust the current workspace and requests.",
  },
];

export function ApprovalsPanel({ config, pending, onRefresh, onSetCommandTrustMode }: Props) {
  return (
    <section className="liquid-glass rounded-3xl p-5 sm:p-6">
      <div className="flex items-center justify-between gap-4">
        <div><p className="eyebrow">Safety</p><h2 className="mt-1 font-display text-xl font-semibold text-ink">Command approvals</h2></div>
        <button className="secondary-button min-h-0 px-3 py-2 text-xs" onClick={onRefresh}><RefreshCw className="h-3.5 w-3.5" />Refresh</button>
      </div>

      <div className="mt-5 grid gap-3 md:grid-cols-3">
        {commandModes.map((option) => {
          const selected = config.commandTrustMode === option.mode;
          return (
            <label key={option.mode} className={`flex cursor-pointer items-start gap-3 rounded-2xl border p-4 transition ${selected ? "border-moss bg-sage-soft" : "border-edge bg-surface-strong/60 hover:bg-surface-strong"}`}>
              <input className="mt-0.5 h-4 w-4 accent-moss" type="radio" name="command-trust-mode" checked={selected} onChange={() => onSetCommandTrustMode(option.mode)} />
              <span>
                <strong className="block text-sm text-ink">{option.title}</strong>
                <small className="mt-1 block text-xs leading-5 text-ink-soft">{option.description}</small>
              </span>
            </label>
          );
        })}
      </div>

      {/* Pending local-command approvals now live in the same queue as email/calendar/GitHub
          actions — /app/approvals is the one place to decide on any of them, from either this
          device or the web. This panel stays for trust-mode config and an offline fallback note. */}
      <div className="mt-4">
        {pending.length === 0 ? (
          <div className="rounded-2xl border border-dashed border-edge p-6 text-center">
            {config.commandTrustMode === "all" ? <ShieldAlert className="mx-auto h-7 w-7 text-gold" /> : config.commandTrustMode === "trusted_coding" ? <ShieldCheck className="mx-auto h-7 w-7 text-moss" /> : <CheckCircle2 className="mx-auto h-7 w-7 text-moss" />}
            <p className="mt-2 text-sm font-medium text-ink-soft">{config.commandTrustMode === "all" ? "All command requests can run automatically." : config.commandTrustMode === "trusted_coding" ? "Recognized coding commands can run automatically." : "Nothing waiting for approval."}</p>
          </div>
        ) : (
          <Link href="/app/approvals" className="flex items-center justify-between gap-3 rounded-2xl border border-edge bg-sage-soft p-4 transition hover:border-moss/50">
            <span>
              <strong className="block text-sm text-ink">{pending.length} command{pending.length === 1 ? "" : "s"} waiting on you</strong>
              <small className="mt-1 block text-xs leading-5 text-ink-soft">Review and decide in the approvals queue.</small>
            </span>
            <ArrowUpRight className="h-4 w-4 shrink-0 text-moss" />
          </Link>
        )}
      </div>
    </section>
  );
}
