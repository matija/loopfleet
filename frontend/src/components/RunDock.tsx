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
import { normalizeDisplayText } from "../displayText";
import { RUN_STATUS_ICON, RUN_STATUS_LABEL, isActiveRun } from "../status";
import { formatDuration } from "./DataGrid";
import { Elapsed } from "./Elapsed";
import {
  AgentIcon,
  BoxIcon,
  ClockIcon,
  FolderIcon,
  GitBranchIcon,
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
};

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

function RunChip({
  run: r,
  selected,
  finishedAt,
  onOpen,
  onStop,
  onDismiss,
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
}) {
  const active = isActiveRun(r.status);
  const taskText = normalizeDisplayText(r.taskText);
  const StatusIcon = RUN_STATUS_ICON[r.status];
  const anchorRef = useRef<HTMLButtonElement>(null);
  // `onClose` deliberately no-ops rather than reusing `useHoverOpen`'s
  // close(): Popover returns focus to its anchor whenever it closes, which
  // would steal focus back to this chip on every ordinary mouseleave. Leave
  // and Escape are already handled by the hover handlers below.
  const { open, handlers } = useHoverOpen();

  return (
    <li
      className={`run-chip${selected ? " run-chip--selected" : ""}${r.unseen ? " run-chip--unseen" : ""}`}
    >
      <button
        ref={anchorRef}
        className="run-chip__open"
        aria-current={selected}
        onClick={() => onOpen(r.runId)}
        title={r.unseen ? `${taskText} — finished, not yet seen` : taskText}
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
        <span className="run-chip__meta">
          {r.agent} · {r.projectName}
          {active && <> · <Elapsed startedAt={r.startedAt} /></>}
        </span>
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
      {active ? (
        <button
          className="run-chip__action"
          onClick={() => onStop(r.runId)}
          title="Stop at the next pass boundary"
          aria-label="Stop run"
        >
          <SquareIcon size={14} />
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
  selectedRunId,
  onOpen,
  onStop,
  onDismiss,
  collapsed,
}: {
  runs: ActiveRun[];
  selectedRunId: string | null;
  onOpen: (runId: string) => void;
  onStop: (runId: string) => void;
  onDismiss: (runId: string) => void;
  /// Collapsed to just the head strip via the toolbar's panel-bottom toggle.
  /// App.tsx owns the persisted state; the dock just renders it.
  collapsed?: boolean;
}) {
  const activeCount = runs.filter((r) => isActiveRun(r.status)).length;
  const unseenCount = runs.filter((r) => r.unseen).length;
  // Idle (no run launched this session yet) collapses the dock to the head
  // strip the same way the manual toggle does — there is nothing to show.
  const idle = runs.length === 0;
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
          {activeCount} active{runs.length > activeCount ? ` · ${runs.length - activeCount} finished` : ""}
          {unseenCount > 0 ? ` · ${unseenCount} new` : ""}
        </span>
      </div>
      {effectiveCollapsed ? null : (
        <ul className="run-dock__list">
          {runs.map((r) => (
            <RunChip
              key={r.runId}
              run={r}
              selected={r.runId === selectedRunId}
              finishedAt={finishedAtRef.current.get(r.runId)}
              onOpen={onOpen}
              onStop={onStop}
              onDismiss={onDismiss}
            />
          ))}
        </ul>
      )}
    </section>
  );
}
