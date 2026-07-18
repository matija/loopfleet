// Compare view (PRD M7): every run bound to one task, side by side, each with
// the diff it produced against its base (final shadow ref). The app never scores
// or judges — it shows what each run made. "Use this run" merges the chosen run's
// branch into the repo's current branch by default (a descriptive merge commit),
// or into a custom branch you name, and marks the run accepted. Read-only
// otherwise.
//
// Reuses the Diff component from the run timeline (same per-file summary +
// collapsible patch) and the run-view header/status vocabulary. Consumes only
// pre-existing commands: `compare_task` and `use_run`.

import { useCallback, useEffect, useState } from "react";
import { compareTask } from "../commands";
import { normalizeDisplayText } from "../displayText";
import type {
  CompareView as Compare,
  RunCompare,
  RunStatus,
} from "../types";
import { Diff } from "./RunTimeline";
import { UseRun } from "./UseRun";

const STATUS_LABEL: Record<RunStatus, string> = {
  queued: "Queued",
  running: "Running",
  completed: "Completed",
  failed: "Failed",
  stopped: "Stopped",
};

export function CompareView({
  planId,
  taskAnchor,
  taskText,
  onClose,
  onAccepted,
}: {
  planId: string;
  taskAnchor: string;
  taskText: string;
  onClose: () => void;
  // Called after a run is accepted so the plan's derived status can refresh.
  onAccepted: () => void;
}) {
  const [compare, setCompare] = useState<Compare | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(() => {
    compareTask(planId, taskAnchor)
      .then(setCompare)
      .catch((e) => setError(String(e)));
  }, [planId, taskAnchor]);

  useEffect(() => {
    setCompare(null);
    setError(null);
    reload();
  }, [reload]);

  return (
    <section className="run-view">
      <header className="run-view__head">
        <button className="run-view__back" onClick={onClose}>
          ← Back
        </button>
        <div className="run-view__ident">
          <span
            className="run-view__task"
            title={normalizeDisplayText(taskText)}
          >
            {normalizeDisplayText(taskText)}
          </span>
          <span className="run-view__meta">
            {compare ? compare.runs.length : "…"}{" "}
            {compare && compare.runs.length === 1 ? "run" : "runs"} · compare
          </span>
        </div>
      </header>

      <div className="run-view__stream run-view__stream--full compare">
        {error ? (
          <p className="panel__error">{error}</p>
        ) : !compare ? (
          <p className="run-view__empty">Loading runs…</p>
        ) : compare.runs.length === 0 ? (
          <p className="run-view__empty">
            No runs on this task yet. Launch one from the plan to compare.
          </p>
        ) : (
          <div className="compare__runs">
            {compare.runs.map((run) => (
              <RunColumn
                key={run.run_id}
                run={run}
                onAccepted={() => {
                  reload();
                  onAccepted();
                }}
              />
            ))}
          </div>
        )}
      </div>
    </section>
  );
}

// One run's column: identity, status, its produced diff, and the "use this run"
// merge control.
function RunColumn({
  run,
  onAccepted,
}: {
  run: RunCompare;
  onAccepted: () => void;
}) {
  return (
    <div className={`compare__run${run.accepted ? " compare__run--accepted" : ""}`}>
      <header className="compare__run-head">
        <span className={`run-view__status run-view__status--${run.status}`}>
          {STATUS_LABEL[run.status]}
        </span>
        <span className="compare__run-id" title={run.run_id}>
          {run.run_id.slice(0, 8)}
        </span>
        <span className="compare__run-agent">{run.agent}</span>
        {run.accepted && <span className="compare__accepted">✓ accepted</span>}
      </header>

      {run.final_ref ? (
        <Diff diff={run.diff} />
      ) : (
        <p className="timeline__no-diff">
          This run produced no snapshot — nothing to diff.
        </p>
      )}

      <UseRun
        runId={run.run_id}
        mergeable={run.final_ref !== null}
        onAccepted={onAccepted}
      />
    </div>
  );
}
