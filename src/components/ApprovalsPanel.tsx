import React from "react";
import { Play, RefreshCw, X } from "lucide-react";
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
    <div className="panel">
      <div className="panel-header">
        <h2>Command approvals</h2>
        <button className="icon-button" onClick={onRefresh} title="Refresh">
          <RefreshCw size={14} />
        </button>
      </div>

      <label className="toggle-row">
        <input
          type="checkbox"
          checked={config.alwaysAllowCommands}
          onChange={(e) => onToggleAlwaysAllow(e.target.checked)}
        />
        <span>
          <strong>Always allow commands</strong>
          <small>Run command requests inside granted folders without asking each time.</small>
        </span>
      </label>

      <div className="approval-list" style={{ marginTop: 12 }}>
        {config.alwaysAllowCommands && pending.length === 0 && (
          <p className="muted">Command requests will run automatically.</p>
        )}
        {!config.alwaysAllowCommands && pending.length === 0 && (
          <p className="muted">No commands waiting for approval.</p>
        )}
        {pending.map((item) => (
          <div className="approval-row" key={item.jobId}>
            <div>
              <p className="reason">{item.reason}</p>
              <code>{item.command}</code>
              <span>{item.cwd}</span>
            </div>
            <div className="actions">
              <button className="primary" onClick={() => onApprove(item.jobId, true)}>
                <Play size={13} /> Run
              </button>
              <button className="secondary" onClick={() => onApprove(item.jobId, false)}>
                <X size={13} /> Deny
              </button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
