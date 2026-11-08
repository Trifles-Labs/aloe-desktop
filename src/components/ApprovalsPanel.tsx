import React, { useId, useState } from "react";
import Link from "next/link";
import { ArrowUpRight, CheckCircle2, RefreshCw, ShieldAlert, ShieldCheck } from "lucide-react";

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
    mode: "auto",
    title: "Auto",
    /* No local override sits in front of the judge: `check_command_safety` (executor.rs) is the
       whole gate in this mode, so the copy must not promise a second one. A declined command is
       refused outright rather than queued; only a check that could not run falls back to asking. */
    description:
      "Aloe asks its own model to judge each command against the conversation before running it. A command it declines does not run; if the check cannot be reached, the command waits for you instead.",
  },
  {
    mode: "all",
    title: "Allow all commands",
    description: "Every command runs immediately with your OS permissions.",
  },
];

export function ApprovalsPanel({ config, pending, onRefresh, onSetCommandTrustMode }: Props) {
  const [refreshing, setRefreshing] = useState(false);
  /* Descriptions are read out after the option's name, not folded into it: without this the
     radio's accessible name is the title and the whole paragraph, which screen readers announce
     as one very long option. */
  const descriptionId = useId();
  const nothingWaiting = pending.length === 0;

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
      /* "Nothing waiting" used to be a fourth row inside the sheet, separated by the same hairline
         as the choices — which read as a fourth option. It is a status line about the queue, so it
         belongs under the group, where the "allow all" warning already lives. */
      footnote={
        config.commandTrustMode === "all" || nothingWaiting ? (
          <span className="flex flex-col gap-1.5">
            {config.commandTrustMode === "all" ? (
              <span className="flex items-start gap-1.5 text-gold">
                <ShieldAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                Aloe can run anything your account can, including deleting files. Use this only in a workspace you fully trust.
              </span>
            ) : null}
            {nothingWaiting ? (
              <span className="flex items-start gap-1.5">
                {config.commandTrustMode === "ask" ? (
                  <CheckCircle2 className="mt-0.5 h-3.5 w-3.5 shrink-0 text-moss" />
                ) : (
                  <ShieldCheck className="mt-0.5 h-3.5 w-3.5 shrink-0 text-moss" />
                )}
                Nothing waiting for approval.
              </span>
            ) : null}
          </span>
        ) : undefined
      }
    >
      <div role="radiogroup" aria-label="Command approvals" className="flex flex-col gap-2 pt-2">
        {commandModes.map((option) => {
          const selected = config.commandTrustMode === option.mode;
          /* Each option is its own card: the background is the card, so hover and
             selection cover the option completely and the gap between cards is the
             separation. A full-width band with rounded corners could only ever cut
             the corners out of the row it sat in. `border-0` drops the hairline a
             `.settings-row` normally takes from its previous sibling. */
          return (
            <label
              key={option.mode}
              className={`settings-row relative cursor-pointer items-start rounded-xl border-0 px-3 transition-colors duration-200 ${
                selected ? "bg-sage-soft" : "hover:bg-sage-soft/40"
              }`}
            >
              <input
                type="radio"
                name="command-trust-mode"
                className="peer sr-only"
                checked={selected}
                aria-label={option.title}
                aria-describedby={`${descriptionId}-${option.mode}`}
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
                <span id={`${descriptionId}-${option.mode}`} className="mt-0.5 block max-w-[62ch] text-xs leading-5 text-ink-soft">
                  {option.description}
                </span>
              </span>
            </label>
          );
        })}
      </div>

      {/* Local commands share the approvals queue with email, calendar and
          GitHub actions — /app/approvals is the one place any of them is
          decided, from this device or the web. */}
      {pending.length > 0 ? (
        <Link href="/app/approvals" className="settings-row press-tap group hover:bg-sage-soft/50">
          <span className="min-w-0">
            <span className="block text-[13px] font-medium leading-5 text-ink">
              {pending.length} command{pending.length === 1 ? "" : "s"} waiting on you
            </span>
            <span className="mt-0.5 block text-xs leading-5 text-ink-soft">Review and decide in the approvals queue.</span>
          </span>
          <ArrowUpRight className="h-4 w-4 shrink-0 text-moss transition-transform duration-150 motion-safe:group-hover:-translate-y-px motion-safe:group-hover:translate-x-px" />
        </Link>
      ) : null}
    </ControlGroup>
  );
}
