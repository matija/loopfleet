// Live run view (PRD M7): the surface the run dock's `onOpen` renders. It
// subscribes to the `run_event` stream for one run, showing the normalized
// events as they arrive, the set of files the run has touched (the app-sourced
// `file_changed` lane), and a Stop button while the run is active.
//
// Scope note: the stream is live-only — `run_event` is not replayed on
// subscribe (see `record_event` in src-tauri), so this view shows events from
// the moment it mounts forward. Historical events (a run opened long after it
// started) are the Run timeline's job (the next M7 task), which reads the
// persisted log via `run_timeline`. Here we stream; there we replay.

import { useEffect, useRef, useState } from "react";
import { onRunEvent } from "../events";
import type { NormalizedEvent, RunStatus } from "../types";
import type { ActiveRun } from "./RunDock";

const ACTIVE: RunStatus[] = ["queued", "running"];

const STATUS_LABEL: Record<RunStatus, string> = {
  queued: "Queued",
  running: "Running",
  completed: "Completed",
  failed: "Failed",
  stopped: "Stopped",
};

type StreamedEvent = { seq: number; event: NormalizedEvent };

export function LiveRunView({
  run,
  onStop,
  onClose,
}: {
  run: ActiveRun;
  onStop: (runId: string) => void;
  onClose: () => void;
}) {
  const [events, setEvents] = useState<StreamedEvent[]>([]);
  // Changed files accumulate as a set (an agent touches a path repeatedly); we
  // keep insertion order for a stable list.
  const [files, setFiles] = useState<string[]>([]);
  const listRef = useRef<HTMLDivElement>(null);

  // One subscription per run. Reset on run change so switching runs starts clean.
  useEffect(() => {
    setEvents([]);
    setFiles([]);
    const un = onRunEvent((p) => {
      if (p.run_id !== run.runId) return;
      if (p.event.kind === "file_changed") {
        const path = p.event.path;
        setFiles((prev) => (prev.includes(path) ? prev : [...prev, path]));
      } else {
        setEvents((prev) => [...prev, { seq: p.seq, event: p.event }]);
      }
    });
    return () => {
      un.then((f) => f());
    };
  }, [run.runId]);

  // Pin the event stream to the newest event as it grows.
  useEffect(() => {
    const el = listRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [events.length]);

  const active = ACTIVE.includes(run.status);

  return (
    <section className="run-view">
      <header className="run-view__head">
        <button className="run-view__back" onClick={onClose}>
          ← Back
        </button>
        <div className="run-view__ident">
          <span className={`run-view__status run-view__status--${run.status}`}>
            {STATUS_LABEL[run.status]}
          </span>
          <span className="run-view__task" title={run.taskText}>
            {run.taskText}
          </span>
          <span className="run-view__meta">
            {run.agent} · {run.projectName}
          </span>
        </div>
        {active && (
          <button
            className="run-view__stop"
            onClick={() => onStop(run.runId)}
            title="Stop at the next pass boundary"
          >
            Stop
          </button>
        )}
      </header>

      <div className="run-view__body">
        <div className="run-view__stream" ref={listRef} aria-label="Run events">
          {events.length === 0 ? (
            <p className="run-view__empty">
              {active
                ? "Waiting for events… they stream in as the agent works."
                : "No events streamed while this view was open."}
            </p>
          ) : (
            <ul className="event-list">
              {events.map((e) => (
                <EventRow key={e.seq} event={e.event} />
              ))}
            </ul>
          )}
        </div>

        <aside className="run-view__files" aria-label="Changed files">
          <div className="run-view__files-head">
            Files changed
            <span className="run-view__files-count">{files.length}</span>
          </div>
          {files.length === 0 ? (
            <p className="run-view__empty">
              No file changes observed yet. The app watches the worktree, so this
              catches shell edits too.
            </p>
          ) : (
            <ul className="file-list">
              {files.map((path) => (
                <li key={path} className="file-list__item" title={path}>
                  {path}
                </li>
              ))}
            </ul>
          )}
        </aside>
      </div>
    </section>
  );
}

// One normalized event, rendered by kind. The label is the event vocabulary;
// the body carries the excerpt/text the adapter produced. Exported so the run
// timeline replays the persisted log with the same vocabulary the live stream
// uses. LiveRunView routes `file_changed` to its files panel before calling
// this, so that case only renders in the timeline (which has no side panel).
export function EventRow({ event }: { event: NormalizedEvent }) {
  switch (event.kind) {
    case "turn_started":
      return <Row kind="turn" label="Turn started" />;
    case "assistant_text":
      return <Row kind="text" label="Assistant" body={event.text} />;
    case "reasoning":
      return <Row kind="reasoning" label="Reasoning" body={event.text} />;
    case "tool_call":
      return (
        <Row
          kind="tool"
          label={`Tool · ${event.name}`}
          body={event.input_excerpt}
        />
      );
    case "tool_result":
      return (
        <Row
          kind={event.ok ? "tool" : "error"}
          label={event.ok ? "Tool result" : "Tool result · error"}
          body={event.output_excerpt}
        />
      );
    case "command_run":
      return (
        <Row
          kind="command"
          label={
            event.exit === null ? "Command" : `Command · exit ${event.exit}`
          }
          body={event.cmd}
        />
      );
    case "turn_completed":
      return (
        <Row
          kind="turn"
          label="Turn completed"
          body={`${event.usage.input_tokens} in · ${event.usage.output_tokens} out tokens`}
        />
      );
    case "needs_approval":
      return <Row kind="warn" label="Needs approval" />;
    case "failed":
      return <Row kind="error" label="Failed" body={event.reason} />;
    case "ended":
      return <Row kind="turn" label="Ended" />;
    case "file_changed":
      // In the live view this is routed to the files panel and never reaches
      // here; in the timeline (no side panel) it renders inline.
      return <Row kind="file" label="File changed" body={event.path} />;
  }
}

function Row({
  kind,
  label,
  body,
}: {
  kind: string;
  label: string;
  body?: string;
}) {
  return (
    <li className={`event-row event-row--${kind}`}>
      <span className="event-row__label">{label}</span>
      {body && <span className="event-row__body">{body}</span>}
    </li>
  );
}
