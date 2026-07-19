// Run timeline (PRD M7): a run's iterations as rows, each with the normalized
// events that occurred during it and the diff that iteration produced. Unlike
// the live view (which streams `run_event` and shows only what arrives while it
// is open), this replays the *persisted* log via `run_timeline`, so it is the
// surface for inspecting a run after it has ended. The app is read-only here.

import { useEffect, useState } from "react";
import { runTimeline } from "../commands";
import { normalizeDisplayText } from "../displayText";
import type {
  DiffView,
  IterationView,
  RunStatus,
  RunTimeline as Timeline,
} from "../types";
import type { ActiveRun } from "./RunDock";
import { DataGrid, formatDuration, GridFooter, rowsDuration } from "./DataGrid";
import { RunSubtabs, type RunSubtab } from "./RunSubtabs";
import { UseRun } from "./UseRun";
import { RUN_STATUS_LABEL, isActiveRun } from "../status";

export function RunTimeline({
  run,
  onClose,
  onAccepted,
  onRetry,
}: {
  run: ActiveRun;
  onClose: () => void;
  // Called after a run is accepted from here so any open plan can refresh its
  // derived status. Optional — the timeline is reachable without a plan open.
  onAccepted?: () => void;
  // Re-launch this run's task (same agent + passes). Backs the rate-limited
  // banner's "Retry now"; a no-op when the run carries no project/task identity.
  onRetry?: (run: ActiveRun) => void;
}) {
  const [timeline, setTimeline] = useState<Timeline | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The Events / Diff / Files subtab. All three panels stay mounted (toggled
  // with `hidden`) so switching preserves each panel's scroll position.
  const [subtab, setSubtab] = useState<RunSubtab>("events");
  // Events-subtab iteration paging: one iteration's grid at a time, navigated
  // via the grid footer's Prev/Next (the DB-client `Prev/Next` analog). The
  // Diff subtab keeps its stacked per-iteration layout — paging is a grid-only
  // affordance, so the two stay intentionally independent.
  const [iterPage, setIterPage] = useState(0);

  useEffect(() => {
    setTimeline(null);
    setError(null);
    setIterPage(0);
    runTimeline(run.runId)
      .then(setTimeline)
      .catch((e) => setError(String(e)));
  }, [run.runId]);

  // Prefer the persisted status once loaded; fall back to the dock's view.
  const status = (timeline?.status as RunStatus) ?? run.status;
  const iterations = timeline?.iterations ?? [];
  const eventCount = iterations.reduce((n, it) => n + it.events.length, 0);
  // Passes the run was launched with (from the persisted timeline, falling back
  // to the dock's seed) and whether it produced a snapshot to merge.
  const passes = timeline?.max_iterations ?? run.maxIterations;
  const passLabel =
    passes !== undefined
      ? `${passes} ${passes === 1 ? "pass" : "passes"}`
      : null;
  const mergeable = iterations.some((it) => it.shadow_ref !== null);
  const canUse = !isActiveRun(status) && mergeable;
  // A rate-limited run gets an explicit banner: what happened, when the limit
  // resets (from the persisted `rate_limited` event), and a way to re-run now.
  const limitReached = status === "limit-reached";
  const canRetry = Boolean(onRetry && run.projectId && run.taskAnchor);
  // Clamped events-subtab page: a stale iterPage (after a reload yields fewer
  // iterations) must never index off the end.
  const page = iterations.length ? Math.min(iterPage, iterations.length - 1) : 0;

  return (
    <section className="run-view">
      <header className="run-view__head">
        <button className="run-view__back" onClick={onClose}>
          ← Back
        </button>
        <div className="run-view__ident">
          <span className={`run-view__status status-pill status-pill--${status}`}>
            {RUN_STATUS_LABEL[status]}
          </span>
          <span
            className="run-view__task"
            title={normalizeDisplayText(run.taskText)}
          >
            {normalizeDisplayText(run.taskText)}
          </span>
          <span className="run-view__meta">
            <span className="run-view__agent">{run.agent}</span>
            {passLabel && <> · {passLabel}</>} · {run.projectName}
          </span>
        </div>
      </header>

      {limitReached && (
        <div className="run-view__limit" role="status">
          <div className="run-view__limit-body">
            <span className="run-view__limit-head">Rate limit reached</span>
            <span className="run-view__limit-detail">
              The agent hit its rate limit, so the run stopped early to wait it
              out. {resetPhrase(limitResetAt(iterations))}
            </span>
          </div>
          {canRetry && (
            <button
              className="run-view__retry"
              onClick={() => onRetry?.(run)}
              title="Re-run this task now with the same agent and passes"
            >
              Retry now
            </button>
          )}
        </div>
      )}

      {canUse && (
        <div className="run-view__use">
          <UseRun
            runId={run.runId}
            mergeable={mergeable}
            onAccepted={() => onAccepted?.()}
          />
        </div>
      )}

      {error ? (
        <div className="run-view__stream run-view__stream--full">
          <p className="panel__error">{error}</p>
        </div>
      ) : !timeline ? (
        <div className="run-view__stream run-view__stream--full">
          <p className="run-view__empty">Loading timeline…</p>
        </div>
      ) : (
        <>
          <RunSubtabs
            active={subtab}
            onSelect={setSubtab}
            counts={{
              events: eventCount,
              diff: iterations.length,
            }}
          />

          <div className="run-view__panels">
            <div
              className="run-view__stream run-view__stream--events"
              hidden={subtab !== "events"}
              aria-label="Run events"
            >
              {iterations.length === 0 ? (
                <div className="run-view__grid-scroll">
                  <p className="run-view__empty">
                    This run recorded no iterations. Nothing was snapshotted.
                  </p>
                </div>
              ) : (
                <>
                  <div className="run-view__grid-scroll">
                    {iterations[page].events.length > 0 ? (
                      <DataGrid
                        rows={iterations[page].events.map((e) => ({
                          seq: e.seq,
                          ts: e.ts,
                          event: e.event,
                        }))}
                      />
                    ) : (
                      <p className="timeline__no-diff">
                        No events this iteration.
                      </p>
                    )}
                  </div>
                  <GridFooter
                    count={iterations[page].events.length}
                    duration={formatDuration(
                      rowsDuration(
                        iterations[page].events.map((e) => ({
                          seq: e.seq,
                          ts: e.ts,
                          event: e.event,
                        })),
                      ),
                    )}
                    iterLabel={`Iteration ${iterations[page].n} of ${
                      iterations[iterations.length - 1].n
                    }`}
                    paging={{
                      onPrev: () => setIterPage((p) => Math.max(0, p - 1)),
                      onNext: () =>
                        setIterPage((p) => Math.min(iterations.length - 1, p + 1)),
                      prevDisabled: page === 0,
                      nextDisabled: page === iterations.length - 1,
                    }}
                  />
                </>
              )}
            </div>

            <div
              className="run-view__stream"
              hidden={subtab !== "diff"}
              aria-label="Run diffs"
            >
              {iterations.length === 0 ? (
                <p className="run-view__empty">
                  This run recorded no iterations. Nothing was snapshotted.
                </p>
              ) : (
                <ol className="timeline">
                  {iterations.map((it) => (
                    <IterationDiff key={it.n} iteration={it} />
                  ))}
                </ol>
              )}
            </div>
          </div>
        </>
      )}
    </section>
  );
}

// The most recent rate-limit reset time in a run's persisted log (ISO-8601), or
// null if none was recorded / none was reported. Scans newest-first so the last
// limit the run hit wins.
function limitResetAt(iterations: IterationView[]): string | null {
  for (let i = iterations.length - 1; i >= 0; i--) {
    const events = iterations[i].events;
    for (let j = events.length - 1; j >= 0; j--) {
      const ev = events[j].event;
      if (ev.kind === "rate_limited") return ev.reset_at;
    }
  }
  return null;
}

// A plain-language sentence for when the limit lifts, from an ISO-8601 reset
// time. Handles no/unparseable time (the agent didn't report one) and a reset
// already in the past (the limit has since cleared).
function resetPhrase(resetAt: string | null): string {
  if (!resetAt) return "It reported no reset time — retry when you're ready.";
  const t = new Date(resetAt);
  if (Number.isNaN(t.getTime())) {
    return "It reported no reset time — retry when you're ready.";
  }
  const at = t.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const mins = Math.round((t.getTime() - Date.now()) / 60000);
  if (mins <= 0) return `The limit reset around ${at}.`;
  const rel =
    mins < 60 ? `${mins} min` : `${Math.floor(mins / 60)}h ${mins % 60}m`;
  return `The limit resets in ~${rel} (around ${at}).`;
}

// One iteration's diff (per-file summary + collapsible patch) under the Diff
// subtab. The heavy patch text stays behind the `Diff` toggle.
function IterationDiff({ iteration }: { iteration: IterationView }) {
  const changed = iteration.diff?.files.length ?? 0;
  return (
    <li className="timeline__iter">
      <div className="timeline__iter-head">
        <span className="timeline__iter-n">Iteration {iteration.n}</span>
        <span className="timeline__iter-meta">
          {changed} {changed === 1 ? "file" : "files"}
        </span>
      </div>
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
// Exported so the PRD edit review renders its computed patch the same way.
export function Patch({ text }: { text: string }) {
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
