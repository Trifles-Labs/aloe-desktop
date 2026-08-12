import React, { useState } from "react";
import { Check, Copy, Laptop, LogOut, Radio, UserRound } from "lucide-react";

import Button from "@/components/ui/Button";
import Modal from "@/components/ui/Modal";
import type { AgentConfig } from "../types";
import { copyText } from "../lib/desktop";
import { ControlGroup, ControlRow } from "./ControlGroup";

type Props = { config: AgentConfig; onReset: () => void };

function CopyButton({ value, label }: { value: string; label: string }) {
  const [copied, setCopied] = useState(false);

  const copy = async () => {
    if (!(await copyText(value))) return;
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1600);
  };

  return (
    <button
      type="button"
      onClick={() => void copy()}
      title={label}
      aria-label={label}
      className="press-tap inline-flex h-8 w-8 items-center justify-center rounded-lg border border-edge bg-surface-strong text-ink-soft hover:bg-sage-soft hover:text-ink"
    >
      {/* The tick is the whole confirmation — a toast for a copy is noise. */}
      {copied ? <Check className="h-3.5 w-3.5 text-moss" /> : <Copy className="h-3.5 w-3.5" />}
    </button>
  );
}

export function ConnectionPanel({ config, onReset }: Props) {
  const [confirming, setConfirming] = useState(false);
  const agentId = config.agentId ?? "";

  return (
    <>
      <ControlGroup
        label="Connection"
        action={
          <Button variant="ghost" size="sm" className="press-tap -mb-1 text-xs" onClick={() => setConfirming(true)}>
            <LogOut className="h-3.5 w-3.5" />
            Log out
          </Button>
        }
        footnote={config.socketError ? `Last socket error: ${config.socketError}` : undefined}
      >
        <ControlRow icon={Laptop} title={config.deviceName} detail={config.platform} />
        {config.userProfile ? (
          <ControlRow icon={UserRound} title={config.userProfile.name || "Signed in"} detail={config.userProfile.email} />
        ) : null}
        <ControlRow
          icon={Radio}
          title="Agent ID"
          detail={<span className="block truncate font-mono text-[11px]">{agentId || "Not registered"}</span>}
          control={agentId ? <CopyButton value={agentId} label="Copy agent ID" /> : undefined}
        />
      </ControlGroup>

      {/* Logging out is reversible only by fetching a fresh token from the web
          app, which is exactly the case a confirmation is for. */}
      <Modal open={confirming} onClose={() => setConfirming(false)} size="sm" labelledBy="logout-title">
        <h2 id="logout-title" className="text-lg font-semibold tracking-[-0.01em] text-ink">
          Log this device out?
        </h2>
        <p className="mt-2 text-[13px] leading-6 text-ink-soft">
          Aloe stops reaching this computer — no folder access, no commands, no local search. Pairing it again needs a new setup token.
        </p>
        <div className="mt-5 flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={() => setConfirming(false)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            size="sm"
            onClick={() => {
              setConfirming(false);
              onReset();
            }}
          >
            Log out
          </Button>
        </div>
      </Modal>
    </>
  );
}
