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
import { agentStatus } from "../commands";
import { onRunEvent } from "../events";
import type { AgentStatus } from "../types";
import { RUN_STATUS_LABEL, isActiveRun } from "../status";
import { CommandBar } from "./CommandBar";
import { DataGrid, eventText, formatDuration, rowsDuration, GridFooter, type GridRow } from "./DataGrid";
import { RunSubtabs, type RunSubtab } from "./RunSubtabs";
import type { ActiveRun } from "./RunDock";

export function LiveRunView({
  run,
  onStop,
  onClose,
}: {
  run: ActiveRun;
  onStop: (runId: string) => void;
  onClose: () => void;
}) {
  // The stream carries no persisted ts (see file header), so we stamp arrival
  // time as each event lands — the grid's `ts` column then reflects when it
  // was observed here, matching the timeline's recorded-ts column.
  const [events, setEvents] = useState<GridRow[]>([]);
  // Changed files accumulate as a set (an agent touches a path repeatedly); we
  // keep insertion order for a stable list.
  const [files, setFiles] = useState<string[]>([]);
  // Client-side `WHERE …` event filter and the last-activity stamp the command
  // bar's "Xs ago" freshness ticks against.
  const [filter, setFilter] = useState("");
  const [lastAt, setLastAt] = useState(() => Date.now());
  const [agents, setAgents] = useState<AgentStatus[]>([]);
  // The Events / Diff / Files subtab. Panels stay mounted (toggled with
  // `hidden`) so switching preserves each panel's scroll — and the one live
  // `run_event` subscription lives on the parent effect, untouched by the switch.
  const [subtab, setSubtab] = useState<RunSubtab>("events");
  const listRef = useRef<HTMLDivElement>(null);

  // One subscription per run. Reset on run change so switching runs starts clean.
  useEffect(() => {
    setEvents([]);
    setFiles([]);
    setFilter("");
    setLastAt(Date.now());
    const un = onRunEvent((p) => {
      if (p.run_id !== run.runId) return;
      setLastAt(Date.now());
      if (p.event.kind === "file_changed") {
        const path = p.event.path;
        setFiles((prev) => (prev.includes(path) ? prev : [...prev, path]));
      } else {
        setEvents((prev) => [
          ...prev,
          { seq: p.seq, ts: Date.now(), event: p.event },
        ]);
      }
    });
    return () => {
      un.then((f) => f());
    };
  }, [run.runId]);

  // Agent availability drives the command bar's Connected/missing pill.
  useEffect(() => {
    agentStatus()
      .then(setAgents)
      .catch(() => {});
  }, []);

  // Pin the event stream to the newest event as it grows, and re-pin when the
  // Events subtab is re-shown (a hidden panel can't scroll while display:none).
  useEffect(() => {
    const el = listRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [events.length, subtab]);

  const active = isActiveRun(run.status);
  // Resolve the agent's human name + detected version so the header states what
  // is actually running, not just the CLI key. (Model/effort are not tracked by
  // the backend in v1 — the agent identity + version is the run's real "what".)
  const matched = agents.find((a) => a.key === run.agent);
  const agentLabel = matched?.display ?? run.agent;
  const passLabel =
    run.maxIterations !== undefined
      ? `${run.maxIterations} ${run.maxIterations === 1 ? "pass" : "passes"}`
      : null;
  const q = filter.trim().toLowerCase();
  const shown = q
    ? events.filter((e) => eventText(e.event).toLowerCase().includes(q))
    : events;

  return (
    <section className="run-view">
      <header className="run-view__head">
        <button className="run-view__back" onClick={onClose}>
          ← Back
        </button>
        <div className="run-view__ident">
          <span className={`run-view__status status-pill status-pill--${run.status}`}>
            {RUN_STATUS_LABEL[run.status]}
          </span>
          <span className="run-view__meta">
            <span className="run-view__agent">{agentLabel}</span>
            {matched?.version && (
              <span className="run-view__agent-ver">v{matched.version}</span>
            )}
            {passLabel && <> · {passLabel}</>} · {run.projectName}
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

      <CommandBar
        task={run.taskText}
        filter={{ value: filter, onChange: setFilter }}
        agent={
          agents.length
            ? {
                name: run.agent,
                connected: agents.some(
                  (a) => a.key === run.agent && a.installed,
                ),
              }
            : undefined
        }
        since={active ? lastAt : undefined}
      />

      <RunSubtabs
        active={subtab}
        onSelect={setSubtab}
        counts={{ events: events.length, files: files.length }}
      />

      <div className="run-view__panels">
        <div
          className="run-view__stream run-view__stream--events"
          hidden={subtab !== "events"}
          aria-label="Run events"
        >
          <div className="run-view__grid-scroll" ref={listRef}>
            {events.length === 0 ? (
              <p className="run-view__empty">
                {active
                  ? "Waiting for events… they stream in as the agent works."
                  : "No events streamed while this view was open."}
              </p>
            ) : shown.length === 0 ? (
              <p className="run-view__empty">
                No events match “{filter.trim()}”.
              </p>
            ) : (
              <DataGrid rows={shown} />
            )}
          </div>
          <GridFooter
            count={shown.length}
            duration={formatDuration(rowsDuration(shown))}
          />
        </div>

        <div
          className="run-view__files"
          hidden={subtab !== "files"}
          aria-label="Changed files"
        >
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
        </div>
      </div>
    </section>
  );
}

