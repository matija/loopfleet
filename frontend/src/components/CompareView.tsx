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
import { compareTask, useRun } from "../commands";
import type {
  CompareView as Compare,
  RunCompare,
  RunStatus,
  UseRunResult,
} from "../types";
import { Diff } from "./RunTimeline";

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
          <span className="run-view__task" title={taskText}>
            {taskText}
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

      <UseRun run={run} onAccepted={onAccepted} />
    </div>
  );
}

// "Use this run": by default merge the run's final state into the repo's
// currently checked-out branch under a descriptive commit message. A custom
// target branch is optional — name it only to land the run somewhere other than
// the current branch (created if absent). Never scores or judges runs.
function UseRun({
  run,
  onAccepted,
}: {
  run: RunCompare;
  onAccepted: () => void;
}) {
  const [branch, setBranch] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<UseRunResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Only a run that produced a snapshot can be merged.
  const mergeable = run.final_ref !== null;
  const custom = branch.trim() !== "";

  async function apply() {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const r = await useRun(run.run_id, custom ? branch.trim() : null);
      setResult(r);
      onAccepted();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="use-run">
      <div className="use-run__row">
        <input
          className="use-run__branch"
          type="text"
          placeholder="current branch (optional)"
          value={branch}
          disabled={!mergeable || busy}
          onChange={(e) => setBranch(e.target.value)}
          aria-label="Target branch"
          title="Leave empty to merge into your current branch. Name a branch to land the run elsewhere."
        />
        <button
          className="btn btn--accent use-run__go"
          onClick={apply}
          disabled={!mergeable || busy}
          title={!mergeable ? "No snapshot to merge" : undefined}
        >
          {busy ? "Merging…" : "Use this run"}
        </button>
      </div>
      <p className="use-run__hint">
        {custom
          ? <>Merges into <code>{branch.trim()}</code>.</>
          : <>Merges into your current branch.</>}
      </p>
      {result && (
        <p className="use-run__result">
          Merged into <code>{result.target_branch}</code>{" "}
          {result.up_to_date
            ? "(already up to date)"
            : result.created
              ? "(branch created)"
              : `→ ${result.merged_commit.slice(0, 8)}`}
        </p>
      )}
      {error && <p className="use-run__error">{error}</p>}
    </div>
  );
}
