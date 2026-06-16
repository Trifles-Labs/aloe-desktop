import React, { useState } from "react";
import { ChevronDown, FileText, Folder } from "lucide-react";
import type { RecentAction } from "../types";
import { formatTimestamp, statusMarkClass } from "../types";

// ── JSON display block ────────────────────────────────────────────────────────

function JsonBlock({ label, value }: { label: string; value: unknown }) {
  return (
    <div className="activity-field activity-field-block">
      <dt>{label}</dt>
      <dd>
        <pre className="activity-json">{JSON.stringify(value, null, 2)}</pre>
      </dd>
    </div>
  );
}

// ── Single expandable row ─────────────────────────────────────────────────────

function ActivityRow({ item }: { item: RecentAction }) {
  const [open, setOpen] = useState(false);
  const toggle = () => setOpen((prev) => !prev);

  return (
    <div className="activity-row">
      <div
        className="activity-row-header"
        onClick={toggle}
        onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && toggle()}
        role="button"
        tabIndex={0}
        aria-expanded={open}
      >
        <div className="activity-row-left">
          <span className="activity-kind">{item.kind}</span>
          {!open && item.detail && (
            <span className="activity-detail-preview">{item.detail}</span>
          )}
        </div>
        <div className="activity-row-right">
          <mark className={statusMarkClass(item.status)}>{item.status}</mark>
          <ChevronDown size={14} className={`activity-chevron${open ? " open" : ""}`} />
        </div>
      </div>

      {open && (
        <dl className="activity-expanded">
          <div className="activity-field">
            <dt>When</dt>
            <dd>{formatTimestamp(item.timestamp)}</dd>
          </div>
          <div className="activity-field">
            <dt>Job ID</dt>
            <dd className="mono">{item.jobId}</dd>
          </div>
          {item.detail && (
            <div className="activity-field">
              <dt>Detail</dt>
              <dd className="mono">{item.detail}</dd>
            </div>
          )}
          {item.input != null && <JsonBlock label="Input" value={item.input} />}
          {item.output != null && <JsonBlock label="Output" value={item.output} />}
        </dl>
      )}
    </div>
  );
}

// ── List panel ────────────────────────────────────────────────────────────────

type Props = { actions: RecentAction[] };

export function ActivityList({ actions }: Props) {
  const visible = actions.slice(-50).reverse();

  return (
    <div className="panel">
      <div className="panel-header">
        <h2>Recent activity</h2>
        {actions.length > 0 && (
          <span className="text-muted" style={{ fontSize: 12 }}>
            {actions.length} action{actions.length === 1 ? "" : "s"}
          </span>
        )}
      </div>

      <div className="activity-list">
        {visible.length === 0 ? (
          <p className="muted">Tool calls and local actions will appear here.</p>
        ) : (
          visible.map((item) => (
            <ActivityRow key={`${item.jobId}-${item.timestamp}`} item={item} />
          ))
        )}
      </div>
    </div>
  );
}
