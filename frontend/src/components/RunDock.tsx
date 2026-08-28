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

import {
  useEffect,
  useRef,
  useState,
  type FocusEvent,
  type ReactNode,
  type RefObject,
} from "react";
import type { RunStatus } from "../types";
import { taskSummary } from "../displayText";
import {
  RETRIES_EXHAUSTED_LABEL,
  RUN_STATUS_ICON,
  RUN_STATUS_LABEL,
  canMergeFromDock,
  isActiveRun,
} from "../status";
import { formatDuration } from "./DataGrid";
import { Elapsed } from "./Elapsed";
import {
  AgentIcon,
  BoxIcon,
  CheckIcon,
  ClockIcon,
  FolderIcon,
  GitBranchIcon,
  PlayIcon,
  SquareIcon,
  XIcon,
} from "./Icon";
import { Popover } from "./Popover";

/// One run tracked by the dock. Seeded at launch, its `status` updated from the
/// `run_status` stream.
export type ActiveRun = {
  runId: string;
  projectName: string;
  taskText: string;
  agent: string;
  /// Model override the run was launched with, "" for the agent's own default.
  model?: string;
  /// The project + task this run is bound to — carried so a finished run can be
  /// re-launched (e.g. retrying a rate-limited run). Optional so runs seeded from
  /// sources without the identity still render; retry is gated on both being set.
  projectId?: string;
  taskAnchor?: string;
  /// Max passes the loop was launched with. Optional so runs seeded from
  /// sources without the count still render.
  maxIterations?: number;
  /// Epoch ms captured when the run was launched, so a running run can show how
  /// long it has been going (session-scoped — runs don't survive a restart).
  startedAt: number;
  status: RunStatus;
  /// Set when the run finished while it wasn't the open view — the dock's
  /// attention marker. Cleared by acknowledge-on-focus or opening the run.
  unseen?: boolean;
  /// Set on a `limit-reached` run when the backend has scheduled an automatic
  /// re-run for it (`scheduled_resume`), carrying the epoch ms it fires at.
  /// Cleared when the resume fires or is cancelled — App.tsx owns the timer.
  pendingResume?: { resumeAt: number };
  /// Whether the run has been accepted ("use this run" merged it into a
  /// branch). Read from the run's detail once it reaches a terminal status —
  /// `undefined` while the run is still active or its detail hasn't loaded.
  accepted?: boolean;
  /// Whether the run produced something to merge: at least one iteration has a
  /// shadow ref. Read alongside `accepted` from the run's detail — `undefined`
  /// until then, so "not known yet" stays distinct from "nothing to merge".
  mergeable?: boolean;
  /// Set on a `limit-reached` run when the backend's automatic resume chain hit
  /// its attempt cap: no further re-run is coming, so the chip reads "retries
  /// exhausted" instead of a resume time. Mutually exclusive with
  /// `pendingResume` — exhaustion means nothing was scheduled.
  resumeExhausted?: boolean;
  /// Set on a finished, unaccepted run once the backend arms its auto-merge
  /// countdown (the `auto_merge_armed` event) — cleared on
  /// `auto_merge_cancelled`, `auto_merge_failed`, or the merge itself landing.
  autoMerge?: ArmedAutoMerge;
};

/// An armed auto-merge countdown on a finished run (PRD: Autopilot). `targetBranch`
/// is "" for the repo's currently checked-out branch, same convention as
/// `use_run`'s own `target_branch`. `mergeAt` is epoch ms.
export type ArmedAutoMerge = {
  targetBranch: string;
  mergeAt: number;
};

/// A launch booked for later via `schedule_launch` (the "run when the limit
/// resets" path), not yet fired — seeded from the `scheduled_launch` event
/// (and re-seeded from the same event on a startup rearm), dropped from the
/// dock once `scheduled_launch_fired`/`scheduled_launch_cancelled` lands.
/// App.tsx owns resolving `projectName`/`taskText` from the payload's
/// `plan_id`/`task_anchor` — the dock just renders what it's given.
export type PendingLaunch = {
  id: number;
  projectName: string;
  taskText: string;
  /// Epoch ms the launch is booked to fire at.
  launchAt: number;
  /// What scheduled this launch — `"manual"` (the "run when the limit
  /// resets" path) or `"auto_advance"` (autopilot chaining the plan's next
  /// task after the current run finishes). Mirrors
  /// `ScheduledLaunchPayload.origin`; drives the chip's icon/label and
  /// whether the plan name is surfaced inline instead of only in the tooltip.
  origin: string;
};

/// Live countdown to `target` (epoch ms), ticking once a second — the
/// scheduled-launch counterpart to `Elapsed`. Floors at "now" rather than
/// going negative once the target passes; App.tsx removes the chip once the
/// backend actually confirms the fire, so a brief "now" reads better than a
/// negative duration in the gap.
function Countdown({
  target,
  label = "Time until launch",
}: {
  target: number;
  label?: string;
}) {
  const [, tick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(id);
  }, []);
  const remaining = Math.max(0, target - Date.now());
  return (
    <span className="run-elapsed" aria-label={label}>
      {remaining === 0 ? "now" : `in ${formatDuration(remaining)}`}
    </span>
  );
}

function PendingLaunchChip({
  launch,
  onCancel,
}: {
  launch: PendingLaunch;
  onCancel: (id: number) => void;
}) {
  const taskText = taskSummary(launch.taskText);
  const isAutoAdvance = launch.origin === "auto_advance";
  const statusLabel = isAutoAdvance ? "Auto-advance" : "Scheduled";
  const chipTitle = `${isAutoAdvance ? "Auto-advance — " : ""}${taskText} — ${launch.projectName} — starts ${new Date(
    launch.launchAt,
  ).toLocaleString([], { hour: "numeric", minute: "2-digit" })}`;

  return (
    <li
      className={`run-chip run-chip--pending-launch${isAutoAdvance ? " run-chip--auto-advance" : ""}`}
      title={chipTitle}
    >
      <span className="run-chip__open" aria-disabled="true">
        <span className="run-chip__status run-chip__status--queued" aria-label={statusLabel}>
          {isAutoAdvance ? <PlayIcon size={14} /> : <ClockIcon size={14} />}
        </span>
        <span className="run-chip__task">{taskText}</span>
        <span className="run-chip__meta run-chip__meta--warn">
          {isAutoAdvance ? `${launch.projectName} · ` : ""}
          <Countdown target={launch.launchAt} />
        </span>
      </span>
      <button
        className="run-chip__action run-chip__action--cancel-resume"
        onClick={() => onCancel(launch.id)}
        title={isAutoAdvance ? "Cancel this auto-advance launch" : "Cancel this scheduled launch"}
        aria-label={isAutoAdvance ? "Cancel auto-advance launch" : "Cancel scheduled launch"}
      >
        <XIcon size={14} />
      </button>
    </li>
  );
}

/// Opens `open` after a hover delay, closes it immediately on pointer-leave,
/// and opens/closes on focus/blur too so keyboard users reach the same
/// content a mouse hover would reveal. Shared by every metadata hover card
/// (the dock's run chips here, and PlanView's task rows).
///
/// `containerRef`, when given, is checked on blur: focus moving to another
/// element still inside the container (e.g. a task row's own Run button)
/// keeps the card open instead of flickering closed and reopening.
export function useHoverOpen(
  delayMs = 400,
  containerRef?: RefObject<HTMLElement | null>,
) {
  const [open, setOpen] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  function clearTimer() {
    if (timer.current !== undefined) {
      clearTimeout(timer.current);
      timer.current = undefined;
    }
  }

  useEffect(() => clearTimer, []);

  return {
    open,
    close: () => {
      clearTimer();
      setOpen(false);
    },
    handlers: {
      onMouseEnter: () => {
        clearTimer();
        timer.current = setTimeout(() => setOpen(true), delayMs);
      },
      onMouseLeave: () => {
        clearTimer();
        setOpen(false);
      },
      onFocus: () => {
        clearTimer();
        setOpen(true);
      },
      onBlur: (e: FocusEvent) => {
        const next = e.relatedTarget as Node | null;
        if (next && containerRef?.current?.contains(next)) return;
        clearTimer();
        setOpen(false);
      },
    },
  };
}

/// One aligned icon-plus-value row inside a metadata hover card. `tone`, when
/// given, carries the same strengthened semantic color as `.status-pill` (warn
/// for a review-pending outcome, danger for a failed run) so a row reporting
/// that outcome reads red/amber here too, not only in the dock chip.
export function MetaRow({
  icon,
  value,
  label,
  tone,
}: {
  icon: ReactNode;
  value: ReactNode;
  label: string;
  tone?: "warn" | "danger";
}) {
  return (
    <span className="meta-popover__row">
      <span className="meta-popover__icon" aria-hidden="true">
        {icon}
      </span>
      <span
        className={`meta-popover__value${tone ? ` meta-popover__value--${tone}` : ""}`}
        aria-label={label}
      >
        {value}
      </span>
    </span>
  );
}

/// Maps a finished run's status to the `MetaRow` tone that matches
/// `.status-pill`'s strengthened color for that status — danger for failed,
/// warn for the other non-success terminal states. `undefined` for active or
/// successfully-completed runs, which carry no outcome accent.
export function finishedRunTone(status: RunStatus): "warn" | "danger" | undefined {
  switch (status) {
    case "failed":
      return "danger";
    case "stopped":
    case "limit-reached":
      return "warn";
    default:
      return undefined;
  }
}

/// The branch a run's isolated worktree checks out — deterministic from the
/// run id (`crates/gitx/src/worktree.rs`'s `branch_for`), so it needs no
/// extra field on `ActiveRun`.
export function worktreeBranch(runId: string): string {
  return `agent/${runId}`;
}

/// Whether the chip should wear the merged marker instead of the merge action:
/// the run is finished and its detail says it was already accepted. Kept
/// distinct from `!canMergeFromDock` — that is also false for runs with nothing
/// to merge or a detail that hasn't loaded, neither of which has landed.
export function isMergedRun(run: {
  status: RunStatus;
  accepted?: boolean;
}): boolean {
  return !isActiveRun(run.status) && run.accepted === true;
}

function RunChip({
  run: r,
  selected,
  finishedAt,
  onOpen,
  onStop,
  onDismiss,
  onCancelResume,
  onMerge,
  onCancelAutoMerge,
  onMergeNow,
}: {
  run: ActiveRun;
  selected: boolean;
  /// Epoch ms captured locally the moment the dock observed this run leave
  /// an active status — there is no server-side "finished at" field, so this
  /// is the closest available approximation.
  finishedAt: number | undefined;
  onOpen: (runId: string) => void;
  onStop: (runId: string) => void;
  onDismiss: (runId: string) => void;
  onCancelResume: (runId: string) => void;
  /// Merge this run into the current branch ("use this run" with no target
  /// branch). Resolves once the attempt settles either way — App owns marking
  /// the run accepted and reporting a failure, the chip only shows busy.
  onMerge: (runId: string) => Promise<void>;
  /// Abort an armed auto-merge countdown before it fires (`cancel_auto_merge`).
  /// Optional — omitted while App.tsx doesn't yet track armed countdowns, in
  /// which case a run with `autoMerge` set (none exist yet) renders no button.
  onCancelAutoMerge?: (runId: string) => void;
  /// Fire an armed auto-merge countdown immediately instead of waiting it out.
  /// Resolves once the attempt settles, same contract as `onMerge`.
  onMergeNow?: (runId: string) => Promise<void>;
}) {
  const active = isActiveRun(r.status);
  const taskText = taskSummary(r.taskText);
  const StatusIcon = RUN_STATUS_ICON[r.status];
  // The chip itself carries only the task text, so agent and project — the
  // identity the old two-line chip spelled out — ride the tooltip instead (and
  // the hover card below, for pointer users who linger).
  const chipTitle = [
    taskText,
    `${r.agent} · ${r.projectName}`,
    isMergedRun(r) ? "merged" : undefined,
    r.autoMerge
      ? `auto-merging into ${r.autoMerge.targetBranch || "current branch"}`
      : undefined,
    r.unseen ? "finished, not yet seen" : undefined,
  ]
    .filter(Boolean)
    .join(" — ");
  const anchorRef = useRef<HTMLButtonElement>(null);
  // `onClose` deliberately no-ops rather than reusing `useHoverOpen`'s
  // close(): Popover returns focus to its anchor whenever it closes, which
  // would steal focus back to this chip on every ordinary mouseleave. Leave
  // and Escape are already handled by the hover handlers below.
  const { open, handlers } = useHoverOpen();
  // Local to the chip: a merge in flight. The outcome (accepted, or a toast)
  // arrives back as a prop, so only the pending state lives here.
  const [merging, setMerging] = useState(false);

  async function merge() {
    setMerging(true);
    try {
      await onMerge(r.runId);
    } finally {
      setMerging(false);
    }
  }

  async function mergeNow() {
    if (!onMergeNow) return;
    setMerging(true);
    try {
      await onMergeNow(r.runId);
    } finally {
      setMerging(false);
    }
  }

  return (
    <li
      className={`run-chip${selected ? " run-chip--selected" : ""}${r.unseen ? " run-chip--unseen" : ""}`}
    >
      <button
        ref={anchorRef}
        className="run-chip__open"
        aria-current={selected}
        onClick={() => onOpen(r.runId)}
        title={chipTitle}
        {...handlers}
      >
        {r.unseen && (
          <span
            className="run-chip__unseen"
            aria-label="Finished, not yet seen"
          />
        )}
        <span
          className={`run-chip__status run-chip__status--${r.status}`}
          aria-label={RUN_STATUS_LABEL[r.status]}
        >
          <StatusIcon size={14} />
        </span>
        <span className="run-chip__task">{taskText}</span>
        {r.autoMerge ? (
          <span className="run-chip__meta run-chip__meta--warn">
            {r.autoMerge.targetBranch || "current branch"} ·{" "}
            <Countdown target={r.autoMerge.mergeAt} label="Time until merge" />
          </span>
        ) : r.pendingResume ? (
          <span className="run-chip__meta run-chip__meta--warn">
            Resumes{" "}
            {new Date(r.pendingResume.resumeAt).toLocaleTimeString([], {
              hour: "numeric",
              minute: "2-digit",
            })}
          </span>
        ) : r.resumeExhausted ? (
          <span className="run-chip__meta run-chip__meta--warn">
            {RETRIES_EXHAUSTED_LABEL}
          </span>
        ) : active ? (
          <span className="run-chip__meta">
            <Elapsed startedAt={r.startedAt} />
          </span>
        ) : null}
      </button>
      <Popover
        open={open}
        onClose={() => {}}
        anchorRef={anchorRef}
        role="dialog"
        aria-label={`${taskText} details`}
        className="meta-popover"
      >
        <MetaRow icon={<FolderIcon size={14} />} value={r.projectName} label="Repo" />
        <MetaRow
          icon={<GitBranchIcon size={14} />}
          value={worktreeBranch(r.runId)}
          label="Worktree branch"
        />
        <MetaRow icon={<AgentIcon size={14} />} value={r.agent} label="Agent" />
        <MetaRow
          icon={<BoxIcon size={14} />}
          value={
            r.maxIterations !== undefined
              ? `${r.maxIterations} ${r.maxIterations === 1 ? "pass" : "passes"}`
              : "—"
          }
          label="Pass count"
        />
        <MetaRow
          icon={<ClockIcon size={14} />}
          value={
            active ? (
              <Elapsed startedAt={r.startedAt} />
            ) : finishedAt !== undefined ? (
              `Finished in ${formatDuration(finishedAt - r.startedAt)}`
            ) : (
              RUN_STATUS_LABEL[r.status]
            )
          }
          label="Elapsed or finished time"
          tone={active ? undefined : finishedRunTone(r.status)}
        />
      </Popover>
      {isMergedRun(r) ? (
        <span
          className="run-chip__merged"
          role="img"
          aria-label="Merged"
          title="Already merged into your branch"
        >
          <CheckIcon size={14} />
        </span>
      ) : null}
      {canMergeFromDock(r) && !r.autoMerge && (
        <button
          className="run-chip__action run-chip__action--merge"
          onClick={merge}
          disabled={merging}
          aria-busy={merging}
          title={
            merging ? "Merging…" : "Merge this run into your current branch"
          }
          aria-label={merging ? "Merging run" : "Use this run"}
        >
          <CheckIcon size={14} />
        </button>
      )}
      {r.autoMerge ? (
        <>
          <button
            className="run-chip__action run-chip__action--merge"
            onClick={mergeNow}
            disabled={merging || !onMergeNow}
            aria-busy={merging}
            title={merging ? "Merging…" : "Merge now, skipping the countdown"}
            aria-label={merging ? "Merging run" : "Merge now"}
          >
            <CheckIcon size={14} />
          </button>
          <button
            className="run-chip__action run-chip__action--cancel-resume"
            onClick={() => onCancelAutoMerge?.(r.runId)}
            disabled={!onCancelAutoMerge}
            title="Cancel the scheduled merge"
            aria-label="Cancel scheduled merge"
          >
            <XIcon size={14} />
          </button>
        </>
      ) : active ? (
        <button
          className="run-chip__action"
          onClick={() => onStop(r.runId)}
          title="Stop at the next pass boundary"
          aria-label="Stop run"
        >
          <SquareIcon size={14} />
        </button>
      ) : r.pendingResume ? (
        <button
          className="run-chip__action run-chip__action--cancel-resume"
          onClick={() => onCancelResume(r.runId)}
          title="Cancel the scheduled resume"
          aria-label="Cancel scheduled resume"
        >
          <XIcon size={14} />
        </button>
      ) : (
        <button
          className="run-chip__action run-chip__action--dismiss"
          onClick={() => onDismiss(r.runId)}
          title="Remove from the dock"
          aria-label="Dismiss run"
        >
          <XIcon size={14} />
        </button>
      )}
    </li>
  );
}

export function RunDock({
  runs,
  pendingLaunches = [],
  selectedRunId,
  onOpen,
  onStop,
  onDismiss,
  onCancelResume,
  onCancelLaunch,
  onMerge,
  onCancelAutoMerge,
  onMergeNow,
  collapsed,
}: {
  runs: ActiveRun[];
  /// Launches booked for later that haven't fired yet. Rendered ahead of the
  /// run chips — they aren't runs, so they carry no status/elapsed, only a
  /// countdown and a cancel action.
  pendingLaunches?: PendingLaunch[];
  selectedRunId: string | null;
  onOpen: (runId: string) => void;
  onStop: (runId: string) => void;
  onDismiss: (runId: string) => void;
  onCancelResume: (runId: string) => void;
  /// Abort a pending scheduled launch before it fires (`cancel_scheduled_launch`).
  onCancelLaunch: (id: number) => void;
  /// Merge a finished run into the current branch — offered on chips where
  /// `canMergeFromDock` holds, so landing a good run needs no detour through
  /// the run view. Resolves when the attempt settles, success or failure.
  onMerge: (runId: string) => Promise<void>;
  /// Abort a run's armed auto-merge countdown (`cancel_auto_merge`) — offered
  /// on chips where `autoMerge` is set. Optional until App.tsx wires up the
  /// `auto_merge_armed` event.
  onCancelAutoMerge?: (runId: string) => void;
  /// Fire a run's armed auto-merge countdown immediately, same contract as
  /// `onMerge`. Optional for the same reason as `onCancelAutoMerge`.
  onMergeNow?: (runId: string) => Promise<void>;
  /// Collapsed to just the head strip via the toolbar's panel-bottom toggle.
  /// App.tsx owns the persisted state; the dock just renders it.
  collapsed?: boolean;
}) {
  const activeCount = runs.filter((r) => isActiveRun(r.status)).length;
  const unseenCount = runs.filter((r) => r.unseen).length;
  // Idle (no run launched this session yet, and nothing booked for later)
  // collapses the dock to the head strip the same way the manual toggle
  // does — there is nothing to show.
  const idle = runs.length === 0 && pendingLaunches.length === 0;
  const effectiveCollapsed = collapsed || idle;

  // No server-side "finished at" timestamp rides `ActiveRun` — this captures
  // the moment the dock itself observes each run leave an active status, so
  // the hover card can show a finished duration instead of just re-showing
  // the status label.
  const finishedAtRef = useRef<Map<string, number>>(new Map());
  useEffect(() => {
    for (const r of runs) {
      if (!isActiveRun(r.status) && !finishedAtRef.current.has(r.runId)) {
        finishedAtRef.current.set(r.runId, Date.now());
      }
    }
  }, [runs]);

  return (
    <section
      className={`run-dock${effectiveCollapsed ? " run-dock--collapsed" : ""}`}
      aria-label="Active runs"
    >
      <div className="run-dock__head">
        <span className="run-dock__title">Runs</span>
        <span className="run-dock__count">
          {activeCount} active{runs.length > activeCount ? ` · ${runs.length - activeCount} done` : ""}
          {pendingLaunches.length > 0 ? ` · ${pendingLaunches.length} scheduled` : ""}
          {unseenCount > 0 ? ` · ${unseenCount} new` : ""}
        </span>
      </div>
      {effectiveCollapsed ? null : (
        <ul className="run-dock__list">
          {pendingLaunches.map((p) => (
            <PendingLaunchChip key={`launch-${p.id}`} launch={p} onCancel={onCancelLaunch} />
          ))}
          {runs.map((r) => (
            <RunChip
              key={r.runId}
              run={r}
              selected={r.runId === selectedRunId}
              finishedAt={finishedAtRef.current.get(r.runId)}
              onOpen={onOpen}
              onStop={onStop}
              onDismiss={onDismiss}
              onCancelResume={onCancelResume}
              onMerge={onMerge}
              onCancelAutoMerge={onCancelAutoMerge}
              onMergeNow={onMergeNow}
            />
          ))}
        </ul>
      )}
    </section>
  );
}
