// Plan view: the frozen PRD's task list with a derived `TaskStatus` overlay and
// a launch control on EVERY task (PRD M7). The launch control is deliberately
// decoupled from the authored `checked` flag — `checked` only gates the derived
// status, never the ability to start a run — so a "done" plan (every box checked)
// still shows a Run button per task. Completed-unaccepted tasks are summarized
// in one quiet banner above the list; each affected row carries the amber
// status glyph as its own signal (the compare/accept backlog).

import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { agentStatus, launchRun, listProjects, planOverview } from "../commands";
import { normalizeDisplayText } from "../displayText";
import { onRunStatus } from "../events";
import { readLaunchPrefs, writeLaunchPrefs } from "../launchPrefs";
import { isActiveRun, RUN_STATUS_LABEL } from "../status";
import { SplitButton } from "./Button";
import { formatDuration } from "./DataGrid";
import { Elapsed } from "./Elapsed";
import { NoPlanEmptyState } from "./EmptyState";
import {
  AgentIcon,
  AlertIcon,
  BoxIcon,
  CheckIcon,
  ClockIcon,
  DotIcon,
  FolderIcon,
  GitBranchIcon,
} from "./Icon";
import { Popover } from "./Popover";
import { MetaRow, useHoverOpen, worktreeBranch } from "./RunDock";
import type {
  AgentStatus,
  PlanView as Plan,
  RunStatus,
  TaskStatus,
  TaskView,
} from "../types";

export const STATUS_LABEL: Record<TaskStatus, string> = {
  "not-started": "Not started",
  "in-progress": "In progress",
  "completed-unaccepted": "Needs review",
  accepted: "Accepted",
};

export const STATUS_ICON: Record<TaskStatus, typeof CheckIcon> = {
  "not-started": DotIcon,
  "in-progress": ClockIcon,
  "completed-unaccepted": AlertIcon,
  accepted: CheckIcon,
};

/// What a task launch reports upward for the global run dock.
export type LaunchedRun = {
  runId: string;
  taskText: string;
  taskAnchor: string;
  agent: string;
  maxIterations: number;
};

/// What opening the compare view needs: the plan + task and its display text.
export type CompareTarget = {
  planId: string;
  taskAnchor: string;
  taskText: string;
};

export function PlanView({
  projectId,
  onLaunch,
  onCompare,
}: {
  projectId: string;
  onLaunch: (run: LaunchedRun) => void;
  onCompare: (target: CompareTarget) => void;
}) {
  const [plans, setPlans] = useState<Plan[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [agents, setAgents] = useState<AgentStatus[]>([]);
  const [agentsLoading, setAgentsLoading] = useState(true);
  const [repoName, setRepoName] = useState<string | undefined>(undefined);

  const reload = useCallback(() => {
    planOverview(projectId)
      .then(setPlans)
      .catch((e) => setError(String(e)));
  }, [projectId]);

  // Reset and reload whenever the selected project changes.
  useEffect(() => {
    setPlans(null);
    setError(null);
    reload();
  }, [reload]);

  // The agent menu. Small, stable — fetched once.
  useEffect(() => {
    agentStatus()
      .then(setAgents)
      .catch(() => {})
      .finally(() => setAgentsLoading(false));
  }, []);

  // The project's repo name, for the task rows' metadata hover card — plan
  // overview data doesn't carry it, so it's resolved separately.
  useEffect(() => {
    listProjects()
      .then((ps) => {
        const p = ps.find((x) => x.id === projectId);
        if (p) {
          const parts = p.repo_path.replace(/\/+$/, "").split("/");
          setRepoName(parts[parts.length - 1] || p.repo_path);
        }
      })
      .catch(() => {});
  }, [projectId]);

  const installed = agents.filter((a) => a.installed).map((a) => a.key);

  if (error) return <p className="panel__error">{error}</p>;
  if (!plans) return <p className="plan__loading">Loading plan…</p>;
  if (plans.length === 0) return <NoPlanEmptyState />;

  return (
    <div className="plans">
      {plans.map((plan) => (
        <PlanCard
          key={plan.plan_id}
          plan={plan}
          projectId={projectId}
          repoName={repoName}
          installed={installed}
          agentsLoading={agentsLoading}
          onLaunched={reload}
          onLaunch={onLaunch}
          onCompare={onCompare}
        />
      ))}
    </div>
  );
}

function PlanCard({
  plan,
  projectId,
  repoName,
  installed,
  agentsLoading,
  onLaunched,
  onLaunch,
  onCompare,
}: {
  plan: Plan;
  projectId: string;
  repoName: string | undefined;
  installed: string[];
  agentsLoading: boolean;
  onLaunched: () => void;
  onLaunch: (run: LaunchedRun) => void;
  onCompare: (target: CompareTarget) => void;
}) {
  const review = plan.tasks.filter((t) => t.status === "completed-unaccepted");

  return (
    <section className="plan-card">
      <header className="plan-card__head">
        <h3>{plan.title ?? plan.file_path}</h3>
        <span className="plan-card__path">{plan.file_path}</span>
      </header>

      {review.length > 0 && (
        <div className="review-banner" role="status">
          <AlertIcon size={16} className="review-banner__icon" />
          <span>
            <strong>{review.length}</strong>{" "}
            {review.length === 1 ? "run is" : "runs are"} awaiting review —
            compare the produced diffs and use one, or keep iterating.
          </span>
        </div>
      )}

      {plan.tasks.length === 0 ? (
        <p className="plan-card__empty">
          No tasks in this plan yet — add checklist items to this plan file to
          launch runs against them.
        </p>
      ) : (
        <ul className="task-list">
          {plan.tasks.map((task) => (
            <TaskRow
              key={task.anchor}
              task={task}
              planId={plan.plan_id}
              projectId={projectId}
              repoName={repoName}
              installed={installed}
              agentsLoading={agentsLoading}
              onLaunched={onLaunched}
              onLaunch={onLaunch}
              onCompare={onCompare}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

/// What the row knows about the most recent run it launched this session —
/// enough to populate the metadata hover card. Not persisted; a fresh mount
/// (e.g. reopening the plan) starts with none until the row launches again.
type RowLastRun = {
  runId: string;
  agent: string;
  maxIterations: number;
  startedAt: number;
  status: RunStatus;
  /// Set once the run leaves an active status — there's no server "finished
  /// at" field, so this is the moment the row itself observed the change.
  finishedAt?: number;
};

function TaskRow({
  task,
  planId,
  projectId,
  repoName,
  installed,
  agentsLoading,
  onLaunched,
  onLaunch,
  onCompare,
}: {
  task: TaskView;
  planId: string;
  projectId: string;
  repoName: string | undefined;
  installed: string[];
  agentsLoading: boolean;
  onLaunched: () => void;
  onLaunch: (run: LaunchedRun) => void;
  onCompare: (target: CompareTarget) => void;
}) {
  const StatusIcon = STATUS_ICON[task.status];
  const [lastRun, setLastRun] = useState<RowLastRun | null>(null);
  const rowRef = useRef<HTMLLIElement>(null);
  const { open, handlers } = useHoverOpen(400, rowRef);

  // Track the launched run's terminal transition so the hover card can show
  // a finished duration instead of freezing on "running".
  useEffect(() => {
    if (!lastRun) return;
    const runId = lastRun.runId;
    const unlisten = onRunStatus((p) => {
      if (p.run_id !== runId) return;
      setLastRun((prev) =>
        prev && prev.runId === runId
          ? {
              ...prev,
              status: p.status,
              finishedAt: isActiveRun(p.status) ? undefined : Date.now(),
            }
          : prev,
      );
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [lastRun?.runId]);

  return (
    // tabIndex makes the row itself a keyboard stop so its rest-hidden actions
    // (revealed via :hover / :focus-within in plan.css) are reachable without
    // a pointer.
    <li className="task-row" tabIndex={0} ref={rowRef} {...handlers}>
      <div className="task-row__main">
        <span className={`task-status task-status--${task.status}`}>
          <StatusIcon size={16} className="task-status__icon" />
          <span className="task-status__label">{STATUS_LABEL[task.status]}</span>
        </span>
        <span className="task-row__text">
          {normalizeDisplayText(task.text)}
        </span>
        {task.checked && (
          <span
            className="task-row__checked"
            title="Authored as checked — counts as implemented (Accepted), still runnable."
          >
            authored ✓
          </span>
        )}
        {task.run_count > 0 && (
          <button
            className="task-row__compare"
            onClick={() =>
              onCompare({
                planId,
                taskAnchor: task.anchor,
                taskText: task.text,
              })
            }
            title="Compare this task's runs and use one"
          >
            {/* The count is the always-visible readout; the "compare" cue and
              * chip chrome only surface on row hover/focus (plan.css). */}
            <span className="task-row__run-count">
              {task.run_count} {task.run_count === 1 ? "run" : "runs"}
            </span>
            <span className="task-row__compare-cue">
              {" "}
              · compare
            </span>
          </button>
        )}
      </div>
      <LaunchControl
        projectId={projectId}
        taskAnchor={task.anchor}
        installed={installed}
        agentsLoading={agentsLoading}
        onLaunched={onLaunched}
        onLaunch={(runId, agent, maxIterations) => {
          setLastRun({
            runId,
            agent,
            maxIterations,
            startedAt: Date.now(),
            status: "running",
          });
          onLaunch({
            runId,
            taskText: task.text,
            taskAnchor: task.anchor,
            agent,
            maxIterations,
          });
        }}
      />
      <Popover
        open={open}
        onClose={() => {}}
        anchorRef={rowRef}
        role="dialog"
        aria-label={`${normalizeDisplayText(task.text)} details`}
        className="meta-popover"
      >
        <MetaRow
          icon={<FolderIcon size={14} />}
          value={repoName ?? "—"}
          label="Repo"
        />
        <MetaRow
          icon={<GitBranchIcon size={14} />}
          value={lastRun ? worktreeBranch(lastRun.runId) : "—"}
          label="Worktree branch"
        />
        <MetaRow
          icon={<AgentIcon size={14} />}
          value={lastRun?.agent ?? "—"}
          label="Agent"
        />
        <MetaRow
          icon={<BoxIcon size={14} />}
          value={
            lastRun
              ? `${lastRun.maxIterations} ${lastRun.maxIterations === 1 ? "pass" : "passes"}`
              : "—"
          }
          label="Pass count"
        />
        <MetaRow
          icon={<ClockIcon size={14} />}
          value={
            !lastRun ? (
              "—"
            ) : isActiveRun(lastRun.status) ? (
              <Elapsed startedAt={lastRun.startedAt} />
            ) : lastRun.finishedAt !== undefined ? (
              `Finished in ${formatDuration(lastRun.finishedAt - lastRun.startedAt)}`
            ) : (
              RUN_STATUS_LABEL[lastRun.status]
            )
          }
          label="Elapsed or finished time"
        />
      </Popover>
    </li>
  );
}

export function LaunchControl({
  projectId,
  taskAnchor,
  installed,
  agentsLoading = false,
  onLaunched,
  onLaunch,
  actionsPortal,
}: {
  projectId: string;
  taskAnchor: string;
  installed: string[];
  agentsLoading?: boolean;
  onLaunched: () => void;
  onLaunch: (runId: string, agent: string, maxIterations: number) => void;
  /// When set, the whole control (split button, its menu, and the result
  /// message) portals there instead of rendering inline — the toolbar's
  /// action slot, wired by TaskTab.
  actionsPortal?: HTMLElement | null;
}) {
  // Empty sentinels mean "not chosen yet"; adopted once from launchPrefs
  // after the installed-agents list resolves, then left to the user.
  const [agent, setAgent] = useState<string>("");
  const [passes, setPasses] = useState<number | "">("");
  const [menuOpen, setMenuOpen] = useState(false);
  const [launching, setLaunching] = useState(false);
  const [msg, setMsg] = useState<{ text: string; ok: boolean } | null>(null);
  const initialized = useRef(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const chevronRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (agentsLoading || initialized.current) return;
    initialized.current = true;
    const prefs = readLaunchPrefs(projectId, installed);
    setAgent(prefs.agent);
    setPasses(prefs.passes);
  }, [agentsLoading, installed, projectId]);

  // Persist the user's choices as soon as they diverge from the loaded prefs.
  useEffect(() => {
    if (!initialized.current || agent === "" || passes === "") return;
    writeLaunchPrefs(projectId, { agent, passes });
  }, [projectId, agent, passes]);

  const noAgents = installed.length === 0;
  const passCount = passes || 1;

  function changePasses(delta: number) {
    setPasses(Math.min(50, Math.max(1, passCount + delta)));
  }

  async function launch() {
    setMenuOpen(false);
    setLaunching(true);
    setMsg(null);
    const maxIterations = Math.max(1, passes || 1);
    try {
      const runId = await launchRun({
        projectId,
        taskAnchor,
        agent,
        maxIterations,
      });
      setMsg({ text: "Launched", ok: true });
      onLaunch(runId, agent, maxIterations);
      onLaunched();
    } catch (e) {
      setMsg({ text: String(e), ok: false });
    } finally {
      setLaunching(false);
    }
  }

  // Mirrored onto the DOM so plan.css's `:has(.launch--engaged)` can keep the
  // row visible while the menu/result popover is open — both now portal to
  // document.body via Popover, out of reach of a plain `:has(.launch__menu)`.
  const engaged = menuOpen || msg !== null;

  const content = (
    <div
      className={engaged ? "launch launch--engaged" : "launch"}
      ref={rootRef}
      title={noAgents ? "No agent CLI is installed" : undefined}
    >
      <SplitButton
        className="launch__run"
        onClick={launch}
        onChevronClick={() => setMenuOpen((v) => !v)}
        chevronLabel="Choose agent and passes"
        disabled={noAgents || launching || !agent}
        chevronRef={chevronRef}
      >
        {launching ? "Launching…" : "Run"}
      </SplitButton>
      <Popover
        open={menuOpen}
        onClose={() => setMenuOpen(false)}
        anchorRef={chevronRef}
        placement="bottom-end"
        role="menu"
        aria-label="Choose agent and passes"
        className="launch__menu"
      >
        <div
          className={`launch__agents${agentsLoading ? " launch__agents--loading" : ""}`}
          role="radiogroup"
          aria-label="Agent"
          aria-busy={agentsLoading}
        >
          {agentsLoading
            ? null
            : installed.map((k) => (
                <button
                  key={k}
                  type="button"
                  role="radio"
                  aria-checked={agent === k}
                  className={`launch__agent${agent === k ? " launch__agent--on" : ""}`}
                  data-label={k}
                  disabled={noAgents}
                  onClick={() => setAgent(k)}
                >
                  {k}
                </button>
              ))}
        </div>
        <div className="launch__count" role="group" aria-label="Maximum passes">
          <button
            type="button"
            className="launch__count-btn"
            onClick={() => changePasses(-1)}
            disabled={noAgents || passCount <= 1}
            aria-label="Decrease maximum passes"
          >
            −
          </button>
          <span className="launch__count-value" title="Maximum passes">
            {passCount}
            <span className="launch__count-label">
              {passCount === 1 ? "pass" : "passes"}
            </span>
          </span>
          <button
            type="button"
            className="launch__count-btn"
            onClick={() => changePasses(1)}
            disabled={noAgents || passCount >= 50}
            aria-label="Increase maximum passes"
          >
            +
          </button>
        </div>
      </Popover>
      <Popover
        open={msg !== null}
        onClose={() => setMsg(null)}
        anchorRef={chevronRef}
        placement="bottom-end"
        role="dialog"
        aria-label="Launch result"
        className={
          msg ? `launch__result msg ${msg.ok ? "msg--ok" : "msg--err"}` : "launch__result msg"
        }
      >
        {msg?.text}
      </Popover>
    </div>
  );

  return actionsPortal ? createPortal(content, actionsPortal) : content;
}
