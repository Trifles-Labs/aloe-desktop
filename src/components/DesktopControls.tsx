import React from "react";
import { motion, useReducedMotion } from "framer-motion";
import { MonitorCheck, PlugZap } from "lucide-react";

import PageHeader from "@/components/ui/PageHeader";
import { PANE_IN_REDUCED, paneIn } from "@/lib/motion";
import type { AgentConfig, CommandTrustMode, PendingApproval } from "../types";
import { ActivityList } from "./ActivityList";
import { ApprovalsPanel } from "./ApprovalsPanel";
import { ConnectionPanel } from "./ConnectionPanel";
import { FoldersPanel } from "./FoldersPanel";

type Props = {
  config: AgentConfig;
  pending: PendingApproval[];
  onRefresh: () => void;
  onReset: () => void;
  onAddFolder: () => void;
  onRemoveFolder: (path: string) => void;
  onSetCommandTrustMode: (mode: CommandTrustMode) => void;
};

/* One page, one title, four labelled groups — the same console shape as the web
   app's settings. It used to be four cards, each with its own display heading
   and its own eyebrow, which made a page of five competing titles. */

export function DesktopControls({ config, pending, onRefresh, onReset, onAddFolder, onRemoveFolder, onSetCommandTrustMode }: Props) {
  const reduceMotion = useReducedMotion();
  const connected = config.socketStatus === "connected";

  return (
    <main className="relative h-full overflow-y-auto">
      <div className="mx-auto max-w-3xl px-4 pb-16 sm:px-6">
        <PageHeader
          eyebrow="Aloe Desktop"
          title="Everything Aloe can"
          accent="touch here"
          description={`The local agent on ${config.deviceName}: what it can reach, what it may run without asking, and what it has been doing.`}
          className="py-8 lg:py-10"
          actions={
            <span
              className={`inline-flex items-center gap-2 rounded-full px-3 py-1.5 text-xs font-semibold ${
                connected ? "bg-sage text-ink" : "border border-edge bg-surface text-ink-soft"
              }`}
            >
              <span className={`h-1.5 w-1.5 rounded-full ${connected ? "bg-moss watch-pulse" : "bg-ink-soft/40"}`} />
              <MonitorCheck className="h-3.5 w-3.5" />
              {connected ? "Connected" : config.socketStatus || "Disconnected"}
            </span>
          }
        />

        <motion.div variants={reduceMotion ? PANE_IN_REDUCED : paneIn(1)} initial="hidden" animate="show" className="min-w-0">
          {!connected ? (
            <div className="mb-6 flex items-start gap-3 rounded-xl border border-clay/40 bg-clay/8 px-4 py-3 text-[13px] leading-5 text-danger">
              <PlugZap className="mt-0.5 h-4 w-4 shrink-0" />
              <span>
                The local agent is {config.socketStatus || "disconnected"}. Aloe can't reach this computer until it reconnects.
                {config.socketError ? ` ${config.socketError}` : ""}
              </span>
            </div>
          ) : null}

          <ConnectionPanel config={config} onReset={onReset} />
          <FoldersPanel folders={config.folders} conversationFolders={config.conversationFolders} onAdd={onAddFolder} onRemove={onRemoveFolder} />
          <ApprovalsPanel config={config} pending={pending} onRefresh={onRefresh} onSetCommandTrustMode={onSetCommandTrustMode} />
          <ActivityList actions={config.recentActions} />
        </motion.div>
      </div>
    </main>
  );
}
