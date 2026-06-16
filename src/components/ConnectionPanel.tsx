import React from "react";
import { LogOut } from "lucide-react";
import type { AgentConfig } from "../types";

type Props = {
  config: AgentConfig;
  onReset: () => void;
};

export function ConnectionPanel({ config, onReset }: Props) {
  return (
    <div className="panel">
      <div className="panel-header">
        <h2>Connection</h2>
        <button className="secondary" onClick={onReset}>
          <LogOut size={14} />
          Log out
        </button>
      </div>

      <dl className="meta">
        <div><dt>Device</dt><dd>{config.deviceName}</dd></div>
        <div><dt>Platform</dt><dd>{config.platform}</dd></div>
        <div><dt>Agent ID</dt><dd>{config.agentId ?? "Not registered"}</dd></div>
        <div>
          <dt>Socket</dt>
          <dd>
            {config.socketStatus}
            {config.socketError ? ` — ${config.socketError}` : ""}
          </dd>
        </div>
      </dl>
    </div>
  );
}
