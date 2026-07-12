// Run timeline (PRD M7): a run's iterations as rows, each with the normalized
// events that occurred during it and the diff that iteration produced. Unlike
// the live view (which streams `run_event` and shows only what arrives while it
// is open), this replays the *persisted* log via `run_timeline`, so it is the
// surface for inspecting a run after it has ended. The app is read-only here.

import { useEffect, useState } from "react";
import { runTimeline } from "../commands";
import type {
  DiffView,
  IterationView,
  RunStatus,
  RunTimeline as Timeline,
} from "../types";
import type { ActiveRun } from "./RunDock";
import { DataGrid } from "./DataGrid";

const STATUS_LABEL: Record<RunStatus, string> = {
  queued: "Queued",
  running: "Running",
  completed: "Completed",
  failed: "Failed",
  stopped: "Stopped",
};

export function RunTimeline({
  run,
  onClose,
}: {
  run: ActiveRun;
  onClose: () => void;
}) {
  const [timeline, setTimeline] = useState<Timeline | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setTimeline(null);
    setError(null);
    runTimeline(run.runId)
      .then(setTimeline)
      .catch((e) => setError(String(e)));
  }, [run.runId]);

  // Prefer the persisted status once loaded; fall back to the dock's view.
  const status = (timeline?.status as RunStatus) ?? run.status;

  return (
    <section className="run-view">
      <header className="run-view__head">
        <button className="run-view__back" onClick={onClose}>
          ← Back
        </button>
        <div className="run-view__ident">
          <span className={`run-view__status run-view__status--${status}`}>
            {STATUS_LABEL[status]}
          </span>
          <span className="run-view__task" title={run.taskText}>
            {run.taskText}
          </span>
          <span className="run-view__meta">
            {run.agent} · {run.projectName}
          </span>
        </div>
      </header>

      <div className="run-view__stream run-view__stream--full" aria-label="Run timeline">
        {error ? (
          <p className="panel__error">{error}</p>
        ) : !timeline ? (
          <p className="run-view__empty">Loading timeline…</p>
        ) : timeline.iterations.length === 0 ? (
          <p className="run-view__empty">
            This run recorded no iterations. Nothing was snapshotted.
          </p>
        ) : (
          <ol className="timeline">
            {timeline.iterations.map((it) => (
              <IterationRow key={it.n} iteration={it} />
            ))}
          </ol>
        )}
      </div>
    </section>
  );
}

// One iteration: its events (in log order) and the diff it produced. The patch
// is heavy, so it is collapsed behind a toggle; the per-file summary is always
// shown.
function IterationRow({ iteration }: { iteration: IterationView }) {
  const { files } = iteration.diff ?? { files: [] };
  const changed = files.length;
  return (
    <li className="timeline__iter">
      <div className="timeline__iter-head">
        <span className="timeline__iter-n">Iteration {iteration.n}</span>
        <span className="timeline__iter-meta">
          {iteration.events.length}{" "}
          {iteration.events.length === 1 ? "event" : "events"}
          {changed > 0 && (
            <>
              {" · "}
              {changed} {changed === 1 ? "file" : "files"}
            </>
          )}
        </span>
      </div>

      {iteration.events.length > 0 && (
        <DataGrid
          rows={iteration.events.map((e) => ({
            seq: e.seq,
            ts: e.ts,
            event: e.event,
          }))}
        />
      )}

      <Diff diff={iteration.diff} />
    </li>
  );
}

export function Diff({ diff }: { diff: DiffView | null }) {
  const [showPatch, setShowPatch] = useState(false);

  if (!diff) {
    return (
      <p className="timeline__no-diff">
        No diff for this iteration — the snapshot ref is missing or unreadable.
      </p>
    );
  }
  if (diff.files.length === 0) {
    return <p className="timeline__no-diff">No file changes this iteration.</p>;
  }

  return (
    <div className="diff">
      <ul className="diff__files">
        {diff.files.map((f) => (
          <li key={f.path} className={`diff__file diff__file--${f.status}`}>
            <span className="diff__status" title={f.status}>
              {f.status[0].toUpperCase()}
            </span>
            <span className="diff__path" title={f.old_path ?? f.path}>
              {f.old_path ? `${f.old_path} → ${f.path}` : f.path}
            </span>
            <span className="diff__stat">
              {f.insertions > 0 && (
                <span className="diff__ins">+{f.insertions}</span>
              )}
              {f.deletions > 0 && (
                <span className="diff__del">−{f.deletions}</span>
              )}
            </span>
          </li>
        ))}
      </ul>
      {diff.patch.trim() && (
        <>
          <button
            className="diff__toggle"
            onClick={() => setShowPatch((v) => !v)}
          >
            {showPatch ? "Hide patch" : "Show patch"}
          </button>
          {showPatch && <Patch text={diff.patch} />}
        </>
      )}
    </div>
  );
}

// The unified patch, colored by line origin (+ added, − removed, @ hunk head).
function Patch({ text }: { text: string }) {
  const lines = text.split("\n");
  return (
    <pre className="patch" aria-label="Unified diff">
      {lines.map((line, i) => (
        <span key={i} className={`patch__line patch__line--${lineKind(line)}`}>
          {line + "\n"}
        </span>
      ))}
    </pre>
  );
}

function lineKind(line: string): string {
  if (line.startsWith("+++") || line.startsWith("---")) return "meta";
  if (line.startsWith("@@")) return "hunk";
  if (line.startsWith("+")) return "add";
  if (line.startsWith("-")) return "del";
  return "ctx";
}
