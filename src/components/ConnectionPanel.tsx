import React from "react";
import { Laptop, LogOut, Radio } from "lucide-react";
import type { AgentConfig } from "../types";

type Props = { config: AgentConfig; onReset: () => void };

export function ConnectionPanel({ config, onReset }: Props) {
  return (
    <section className="liquid-glass rounded-3xl p-5 sm:p-6">
      <div className="flex items-center justify-between gap-4">
        <div><p className="eyebrow">Device</p><h2 className="mt-1 font-display text-xl font-semibold text-[#0b3026] dark:text-[#e8f0e0]">Connection</h2></div>
        <button className="secondary-button min-h-0 px-3 py-2 text-xs" onClick={onReset}><LogOut className="h-3.5 w-3.5" />Log out</button>
      </div>
      <dl className="mt-5 space-y-3">
        <div className="flex items-start gap-3 rounded-2xl bg-white/45 p-3 dark:bg-[#152118]/55">
          <Laptop className="mt-0.5 h-4 w-4 text-[#6f8747] dark:text-[#8faa5f]" />
          <div className="min-w-0"><dt className="text-xs text-[#6b786f] dark:text-[#78907e]">Device</dt><dd className="truncate text-sm font-semibold text-[#0b3026] dark:text-[#e8f0e0]">{config.deviceName} · {config.platform}</dd></div>
        </div>
        <div className="flex items-start gap-3 rounded-2xl bg-white/45 p-3 dark:bg-[#152118]/55">
          <Radio className="mt-0.5 h-4 w-4 text-[#6f8747] dark:text-[#8faa5f]" />
          <div className="min-w-0"><dt className="text-xs text-[#6b786f] dark:text-[#78907e]">Agent ID</dt><dd className="truncate font-mono text-xs text-[#506257] dark:text-[#8aaa90]">{config.agentId ?? "Not registered"}</dd></div>
        </div>
      </dl>
    </section>
  );
}
