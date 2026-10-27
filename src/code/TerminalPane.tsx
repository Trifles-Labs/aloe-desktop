/* Terminal: the commands Aloe ran in this session and their output, followed live.
 *
 * Read-only: it shows Aloe's own terminal sessions (terminal.rs), not a shell for the user. Output
 * is the session's retained buffer, which the native side caps, so very long logs lose their start. */

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { CircleCheck, CircleX, Loader2, Square, SquareTerminal } from "lucide-react";

import { cn, relativeTime } from "@aloe/ui/lib/utils";
import { stopTerminal, useTerminalOutput, type TerminalSession } from "./bridge";
import { PaneMessage } from "./ChangesPane";

type Props = {
  conversationId: string | null;
  sessions: TerminalSession[];
  onRefresh: () => void;
};

// eslint-disable-next-line no-control-regex
const ANSI = /\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07]*(\x07|\x1b\\)/g;

/** The buffer tags each chunk "[stdout] " or "[stderr] "; split it back into runs per stream. */
export function splitStreams(output: string): Array<{ stream: "stdout" | "stderr"; text: string }> {
  const runs: Array<{ stream: "stdout" | "stderr"; text: string }> = [];
  const marker = /\[(stdout|stderr)\] /g;
  let stream: "stdout" | "stderr" = "stdout";
  let last = 0;
  for (let match = marker.exec(output); match; match = marker.exec(output)) {
    if (match.index > last) runs.push({ stream, text: output.slice(last, match.index) });
    stream = match[1] as "stdout" | "stderr";
    last = marker.lastIndex;
  }
  if (last < output.length) runs.push({ stream, text: output.slice(last) });
  return runs.map((run) => ({ ...run, text: run.text.replace(ANSI, "").replace(/\r(?!\n)/g, "\n") })).filter((run) => run.text);
}

const isRunning = (session: TerminalSession) => session.status.state === "running";

export default function TerminalPane({ conversationId, sessions, onRefresh }: Props) {
  const [selected, setSelected] = useState<string | null>(null);
  const active = sessions.find((session) => session.sessionId === selected) ?? sessions[0] ?? null;

  /* Follow the newest command while the user hasn't picked one. */
  useEffect(() => {
    if (selected && !sessions.some((session) => session.sessionId === selected)) setSelected(null);
  }, [selected, sessions]);

  if (!conversationId) {
    return <PaneMessage title="No session yet">Commands Aloe runs in this session, like installs, builds and tests, show up here with their output.</PaneMessage>;
  }
  if (sessions.length === 0) {
    return <PaneMessage title="No commands yet">When Aloe runs a command in this session, you can watch its output here as it happens.</PaneMessage>;
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ul className="max-h-[38%] shrink-0 overflow-y-auto border-b border-edge py-1">
        {sessions.map((session) => (
          <li key={session.sessionId}>
            <button
              type="button"
              onClick={() => setSelected(session.sessionId)}
              aria-current={active?.sessionId === session.sessionId ? "true" : undefined}
              className={cn(
                "flex w-full items-center gap-2 px-3 py-1.5 text-left",
                active?.sessionId === session.sessionId ? "bg-sage-soft" : "hover:bg-sage-soft/60",
              )}
            >
              <StatusIcon session={session} />
              <span className="min-w-0 flex-1 truncate font-mono text-xs text-ink" title={session.command}>
                {session.command}
              </span>
              <span className="shrink-0 font-mono text-[10px] text-ink-soft/70">{relativeTime(session.startedAt)}</span>
            </button>
          </li>
        ))}
      </ul>
      {active ? <TerminalOutput key={active.sessionId} session={active} onStopped={onRefresh} /> : null}
    </div>
  );
}

function StatusIcon({ session }: { session: TerminalSession }) {
  if (isRunning(session)) return <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-moss" aria-label="Running" />;
  if (session.status.state === "exited" && session.status.exitCode === 0) return <CircleCheck className="h-3.5 w-3.5 shrink-0 text-moss" aria-label="Succeeded" />;
  if (session.status.state === "exited") return <CircleX className="h-3.5 w-3.5 shrink-0 text-clay" aria-label={`Exited with ${session.status.exitCode}`} />;
  return <Square className="h-3.5 w-3.5 shrink-0 text-ink-soft" aria-label="Stopped" />;
}

function TerminalOutput({ session, onStopped }: { session: TerminalSession; onStopped: () => void }) {
  const running = isRunning(session);
  const { output, status, error } = useTerminalOutput(session.sessionId, running);
  const runs = useMemo(() => splitStreams(output), [output]);
  const [stopping, setStopping] = useState(false);
  const [stopError, setStopError] = useState<string | null>(null);
  const scroller = useRef<HTMLPreElement>(null);
  const following = useRef(true);

  // A session remembered from before a restart says "running" but has no process behind it.
  const live = !error && (status?.state ?? session.status.state) === "running";
  const exitCode = status?.exitCode ?? session.status.exitCode;

  useLayoutEffect(() => {
    const element = scroller.current;
    if (element && following.current) element.scrollTop = element.scrollHeight;
  }, [runs]);

  const stop = async () => {
    setStopping(true);
    try {
      await stopTerminal(session.sessionId);
      onStopped();
    } catch (reason) {
      setStopError(String(reason));
    } finally {
      setStopping(false);
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-2 px-3 py-1.5 text-xs text-ink-soft">
        <SquareTerminal className="h-3.5 w-3.5 shrink-0" />
        <span className="min-w-0 flex-1 truncate font-mono" title={session.cwd}>
          {session.cwd}
        </span>
        {live ? (
          <button
            type="button"
            onClick={() => void stop()}
            disabled={stopping}
            className="inline-flex items-center gap-1 rounded-md bg-clay/10 px-2 py-0.5 font-semibold text-clay hover:bg-clay/15 disabled:opacity-60"
          >
            <Square className="h-3 w-3" />
            Stop
          </button>
        ) : exitCode !== null && exitCode !== undefined ? (
          <span className={cn("font-mono", exitCode === 0 ? "text-moss" : "text-clay")}>exit {exitCode}</span>
        ) : (
          <span>stopped</span>
        )}
      </div>
      {stopError ? <p className="px-3 pb-1 text-xs text-danger">{stopError}</p> : null}
      <pre
        ref={scroller}
        onScroll={(event) => {
          const element = event.currentTarget;
          following.current = element.scrollHeight - element.scrollTop - element.clientHeight < 24;
        }}
        className="mx-3 mb-3 min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words rounded-lg border border-edge bg-surface p-2.5 font-mono text-[11.5px] leading-[1.55] text-ink"
      >
        {error && !output ? (
          <span className="text-ink-soft">Output from before Aloe Desktop last restarted isn&apos;t kept.</span>
        ) : runs.length === 0 ? (
          <span className="text-ink-soft">{live ? "Waiting for output…" : "No output."}</span>
        ) : (
          runs.map((run, index) => (
            <span key={index} className={run.stream === "stderr" ? "text-clay" : undefined}>
              {run.text}
            </span>
          ))
        )}
      </pre>
    </div>
  );
}
