// Global run surface (PRD M7): a persistent dock listing every run launched this
// session, across projects — the always-present "you can run agents here" entry
// point. It lives outside the scrolling main pane, so a launched run stays
// visible and stoppable no matter which project or plan is selected.
//
// Scope note: in v1 runs do not survive an app restart (M6 crash recovery marks
// any still-running run failed on startup, and there is no global "active runs"
// command), so the dock's registry is exactly the runs launched in this session.
// Clicking a run opens its live view — the live view component itself lands in
// the next M7 task; here `onOpen` just carries the selection.

import type { RunStatus } from "../types";
import { normalizeDisplayText } from "../displayText";

/// One run tracked by the dock. Seeded at launch, its `status` updated from the
/// `run_status` stream.
export type ActiveRun = {
  runId: string;
  projectName: string;
  taskText: string;
  agent: string;
  /// Max passes the loop was launched with. Optional so runs seeded from
  /// sources without the count still render.
  maxIterations?: number;
  status: RunStatus;
};

const ACTIVE: RunStatus[] = ["queued", "running"];

const STATUS_LABEL: Record<RunStatus, string> = {
  queued: "Queued",
  running: "Running",
  completed: "Completed",
  failed: "Failed",
  stopped: "Stopped",
};

export function RunDock({
  runs,
  selectedRunId,
  onOpen,
  onStop,
  onDismiss,
}: {
  runs: ActiveRun[];
  selectedRunId: string | null;
  onOpen: (runId: string) => void;
  onStop: (runId: string) => void;
  onDismiss: (runId: string) => void;
}) {
  const activeCount = runs.filter((r) => ACTIVE.includes(r.status)).length;

  return (
    <section className="run-dock" aria-label="Active runs">
      <div className="run-dock__head">
        <span className="run-dock__title">Runs</span>
        <span className="run-dock__count">
          {activeCount} active{runs.length > activeCount ? ` · ${runs.length - activeCount} finished` : ""}
        </span>
      </div>
      {runs.length === 0 ? (
        <p className="run-dock__empty">
          No runs yet. Launch one from a task above — it will appear here and stay
          stoppable while you work.
        </p>
      ) : (
        <ul className="run-dock__list">
          {runs.map((r) => {
            const active = ACTIVE.includes(r.status);
            const taskText = normalizeDisplayText(r.taskText);
            return (
              <li
                key={r.runId}
                className={`run-chip${r.runId === selectedRunId ? " run-chip--selected" : ""}`}
              >
                <button
                  className="run-chip__open"
                  aria-current={r.runId === selectedRunId}
                  onClick={() => onOpen(r.runId)}
                  title={taskText}
                >
                  <span className={`run-chip__status run-chip__status--${r.status}`}>
                    {STATUS_LABEL[r.status]}
                  </span>
                  <span className="run-chip__task">{taskText}</span>
                  <span className="run-chip__meta">
                    {r.agent} · {r.projectName}
                  </span>
                </button>
                {active ? (
                  <button
                    className="run-chip__action"
                    onClick={() => onStop(r.runId)}
                    title="Stop at the next pass boundary"
                  >
                    Stop
                  </button>
                ) : (
                  <button
                    className="run-chip__action run-chip__action--dismiss"
                    onClick={() => onDismiss(r.runId)}
                    title="Remove from the dock"
                    aria-label="Dismiss run"
                  >
                    ✕
                  </button>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
