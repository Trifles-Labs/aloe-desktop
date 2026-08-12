import React, { useState } from "react";
import Link from "next/link";
import { motion, useReducedMotion } from "framer-motion";
import { ArrowUpRight, CheckCircle2, RefreshCw, ShieldAlert, ShieldCheck } from "lucide-react";

import { SPRING_MOVE } from "@/lib/motion";
import type { AgentConfig, CommandTrustMode, PendingApproval } from "../types";
import { ControlGroup } from "./ControlGroup";

type Props = {
  config: AgentConfig;
  pending: PendingApproval[];
  onRefresh: () => void;
  onSetCommandTrustMode: (mode: CommandTrustMode) => void;
};

const commandModes: Array<{ mode: CommandTrustMode; title: string; description: string }> = [
  {
    mode: "ask",
    title: "Ask before every command",
    description: "Nothing runs on this computer until you approve it.",
  },
  {
    mode: "trusted_coding",
    title: "Trusted Coding",
    description: "Recognized inspection, install, build, and test commands run on their own. Anything destructive still waits for you.",
  },
  {
    mode: "all",
    title: "Allow all commands",
    description: "Every command runs immediately with your OS permissions.",
  },
];

export function ApprovalsPanel({ config, pending, onRefresh, onSetCommandTrustMode }: Props) {
  const reduceMotion = useReducedMotion();
  const [refreshing, setRefreshing] = useState(false);

  const refresh = () => {
    // The spin is the acknowledgement — the call itself usually returns before
    // a frame lands, and a control that flashes nothing reads as broken.
    setRefreshing(true);
    window.setTimeout(() => setRefreshing(false), 600);
    onRefresh();
  };

  return (
    <ControlGroup
      label="Command approvals"
      action={
        <button
          type="button"
          onClick={refresh}
          className="press-tap inline-flex items-center gap-1.5 rounded-lg px-2 py-1 text-xs font-semibold text-ink-soft hover:bg-sage-soft hover:text-ink"
        >
          <RefreshCw className={`h-3.5 w-3.5 ${refreshing ? "animate-spin" : ""}`} />
          Refresh
        </button>
      }
      footnote={
        config.commandTrustMode === "all" ? (
          <span className="flex items-start gap-1.5 text-gold">
            <ShieldAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            Aloe can run anything your account can, including deleting files. Use this only in a workspace you fully trust.
          </span>
        ) : undefined
      }
    >
      <div role="radiogroup" aria-label="Command approvals">
        {commandModes.map((option) => {
          const selected = config.commandTrustMode === option.mode;
          return (
            <label key={option.mode} className="settings-row relative cursor-pointer items-start">
              {/* One marker for the group: it travels to the option you picked
                  rather than blinking on there, which is what tells you what you
                  just left. */}
              {selected ? (
                <motion.span
                  aria-hidden
                  layoutId="trust-mode-marker"
                  transition={reduceMotion ? { duration: 0 } : SPRING_MOVE}
                  className="absolute inset-1 rounded-[10px] bg-sage-soft"
                />
              ) : null}

              <input
                type="radio"
                name="command-trust-mode"
                className="peer sr-only"
                checked={selected}
                onChange={() => onSetCommandTrustMode(option.mode)}
              />
              <span
                aria-hidden
                className={`relative mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full border transition-colors duration-150 peer-focus-visible:ring-2 peer-focus-visible:ring-moss/40 ${
                  selected ? "border-moss bg-moss" : "border-edge bg-surface-strong"
                }`}
              >
                <span className={`h-1.5 w-1.5 rounded-full bg-cream transition-transform duration-150 ${selected ? "scale-100" : "scale-0"}`} />
              </span>
              <span className="relative min-w-0 flex-1">
                <span className="block text-[13px] font-medium leading-5 text-ink">{option.title}</span>
                <span className="mt-0.5 block text-xs leading-5 text-ink-soft">{option.description}</span>
              </span>
            </label>
          );
        })}
      </div>

      {/* Local commands share the approvals queue with email, calendar and
          GitHub actions — /app/approvals is the one place any of them is
          decided, from this device or the web. */}
      {pending.length === 0 ? (
        <div className="settings-row">
          <span className="flex items-center gap-3 text-[13px] leading-5 text-ink-soft">
            {config.commandTrustMode === "ask" ? (
              <CheckCircle2 className="h-4 w-4 shrink-0 text-moss" />
            ) : (
              <ShieldCheck className="h-4 w-4 shrink-0 text-moss" />
            )}
            Nothing waiting for approval.
          </span>
        </div>
      ) : (
        <Link href="/app/approvals" className="settings-row press-tap group hover:bg-sage-soft/50">
          <span className="min-w-0">
            <span className="block text-[13px] font-medium leading-5 text-ink">
              {pending.length} command{pending.length === 1 ? "" : "s"} waiting on you
            </span>
            <span className="mt-0.5 block text-xs leading-5 text-ink-soft">Review and decide in the approvals queue.</span>
          </span>
          <ArrowUpRight className="h-4 w-4 shrink-0 text-moss transition-transform duration-150 motion-safe:group-hover:-translate-y-px motion-safe:group-hover:translate-x-px" />
        </Link>
      )}
    </ControlGroup>
  );
}
