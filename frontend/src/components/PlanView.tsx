// Plan view: the frozen PRD's task list with a derived `TaskStatus` overlay and
// a launch control on EVERY task (PRD M7). The launch control is deliberately
// decoupled from the authored `checked` flag — `checked` only gates the derived
// status, never the ability to start a run — so a "done" plan (every box checked)
// still shows a Run button per task. Completed-unaccepted tasks are surfaced
// loudly as a review queue (the compare/accept backlog).

import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { agentStatus, launchRun, planOverview } from "../commands";
import { normalizeDisplayText } from "../displayText";
import { readLaunchPrefs, writeLaunchPrefs } from "../launchPrefs";
import { SplitButton } from "./Button";
import { NoPlanEmptyState } from "./EmptyState";
import { AlertIcon, CheckIcon, ClockIcon, DotIcon } from "./Icon";
import type {
  AgentStatus,
  PlanView as Plan,
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
  installed,
  agentsLoading,
  onLaunched,
  onLaunch,
  onCompare,
}: {
  plan: Plan;
  projectId: string;
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
          <strong>{review.length}</strong>{" "}
          {review.length === 1 ? "run is" : "runs are"} awaiting review — compare
          the produced diffs and use one, or keep iterating.
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

function TaskRow({
  task,
  planId,
  projectId,
  installed,
  agentsLoading,
  onLaunched,
  onLaunch,
  onCompare,
}: {
  task: TaskView;
  planId: string;
  projectId: string;
  installed: string[];
  agentsLoading: boolean;
  onLaunched: () => void;
  onLaunch: (run: LaunchedRun) => void;
  onCompare: (target: CompareTarget) => void;
}) {
  const review = task.status === "completed-unaccepted";
  const StatusIcon = STATUS_ICON[task.status];
  return (
    // tabIndex makes the row itself a keyboard stop so its rest-hidden actions
    // (revealed via :hover / :focus-within in plan.css) are reachable without
    // a pointer.
    <li className={`task-row${review ? " task-row--review" : ""}`} tabIndex={0}>
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
        onLaunch={(runId, agent, maxIterations) =>
          onLaunch({
            runId,
            taskText: task.text,
            taskAnchor: task.anchor,
            agent,
            maxIterations,
          })
        }
      />
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

  useEffect(() => {
    if (!menuOpen) return;
    function onPointerDown(e: MouseEvent) {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    }
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") setMenuOpen(false);
    }
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [menuOpen]);

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

  const content = (
    <div
      className="launch"
      ref={rootRef}
      title={noAgents ? "No agent CLI is installed" : undefined}
    >
      <SplitButton
        className="launch__run"
        onClick={launch}
        onChevronClick={() => setMenuOpen((v) => !v)}
        chevronLabel="Choose agent and passes"
        disabled={noAgents || launching || !agent}
      >
        {launching ? "Launching…" : "Run"}
      </SplitButton>
      {menuOpen && (
        <div className="launch__menu" role="menu">
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
        </div>
      )}
      {msg && (
        <span className={`msg ${msg.ok ? "msg--ok" : "msg--err"}`}>
          {msg.text}
        </span>
      )}
    </div>
  );

  return actionsPortal ? createPortal(content, actionsPortal) : content;
}
