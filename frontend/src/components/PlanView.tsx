// Plan view: the frozen PRD's task list with a derived `TaskStatus` overlay and
// a launch control on EVERY task (PRD M7). The launch control is deliberately
// decoupled from the authored `checked` flag — `checked` only gates the derived
// status, never the ability to start a run — so a "done" plan (every box checked)
// still shows a Run button per task. Completed-unaccepted tasks are surfaced
// loudly as a review queue (the compare/accept backlog).

import { useCallback, useEffect, useState } from "react";
import { agentStatus, getSettings, launchRun, planOverview } from "../commands";
import type {
  AgentStatus,
  PlanView as Plan,
  Settings,
  TaskStatus,
  TaskView,
} from "../types";

export const STATUS_LABEL: Record<TaskStatus, string> = {
  "not-started": "Not started",
  "in-progress": "In progress",
  "completed-unaccepted": "Needs review",
  accepted: "Accepted",
};

/// What a task launch reports upward for the global run dock.
export type LaunchedRun = { runId: string; taskText: string; agent: string };

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
  const [settings, setSettings] = useState<Settings | null>(null);
  const [agents, setAgents] = useState<AgentStatus[]>([]);

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

  // Launch defaults + the agent menu. Small, stable — fetched once.
  useEffect(() => {
    getSettings()
      .then(setSettings)
      .catch(() => {});
    agentStatus()
      .then(setAgents)
      .catch(() => {});
  }, []);

  const installed = agents.filter((a) => a.installed).map((a) => a.key);

  if (error) return <p className="panel__error">{error}</p>;
  if (!plans) return <p className="plan__loading">Loading plan…</p>;
  if (plans.length === 0) {
    return (
      <p className="plan__empty">
        No plan found. Add a <code>PRD.md</code> at the repo root, or a{" "}
        <code>plans/</code> folder of <code>.md</code> files.
      </p>
    );
  }

  return (
    <div className="plans">
      {plans.map((plan) => (
        <PlanCard
          key={plan.plan_id}
          plan={plan}
          projectId={projectId}
          installed={installed}
          settings={settings}
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
  settings,
  onLaunched,
  onLaunch,
  onCompare,
}: {
  plan: Plan;
  projectId: string;
  installed: string[];
  settings: Settings | null;
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

      <ul className="task-list">
        {plan.tasks.map((task) => (
          <TaskRow
            key={task.anchor}
            task={task}
            planId={plan.plan_id}
            projectId={projectId}
            installed={installed}
            settings={settings}
            onLaunched={onLaunched}
            onLaunch={onLaunch}
            onCompare={onCompare}
          />
        ))}
      </ul>
    </section>
  );
}

function TaskRow({
  task,
  planId,
  projectId,
  installed,
  settings,
  onLaunched,
  onLaunch,
  onCompare,
}: {
  task: TaskView;
  planId: string;
  projectId: string;
  installed: string[];
  settings: Settings | null;
  onLaunched: () => void;
  onLaunch: (run: LaunchedRun) => void;
  onCompare: (target: CompareTarget) => void;
}) {
  const review = task.status === "completed-unaccepted";
  return (
    <li className={`task-row${review ? " task-row--review" : ""}`}>
      <div className="task-row__main">
        <span className={`task-badge task-badge--${task.status}`}>
          {STATUS_LABEL[task.status]}
        </span>
        <span className="task-row__text">{task.text}</span>
        {task.checked && (
          <span
            className="task-row__checked"
            title="Authored as checked — excluded from derived status, but still runnable."
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
            {task.run_count} {task.run_count === 1 ? "run" : "runs"} · compare
          </button>
        )}
      </div>
      <LaunchControl
        projectId={projectId}
        taskAnchor={task.anchor}
        installed={installed}
        settings={settings}
        onLaunched={onLaunched}
        onLaunch={(runId, agent) =>
          onLaunch({ runId, taskText: task.text, agent })
        }
      />
    </li>
  );
}

export function LaunchControl({
  projectId,
  taskAnchor,
  installed,
  settings,
  onLaunched,
  onLaunch,
}: {
  projectId: string;
  taskAnchor: string;
  installed: string[];
  settings: Settings | null;
  onLaunched: () => void;
  onLaunch: (runId: string, agent: string) => void;
}) {
  const preferred =
    settings && installed.includes(settings.default_agent)
      ? settings.default_agent
      : installed[0];
  // Empty sentinels mean "not chosen yet"; adopt the resolved defaults once they
  // arrive, then leave the user's choices alone.
  const [agent, setAgent] = useState<string>("");
  const [iterations, setIterations] = useState<number | "">("");
  const [launching, setLaunching] = useState(false);
  const [msg, setMsg] = useState<{ text: string; ok: boolean } | null>(null);

  useEffect(() => {
    if (preferred && agent === "") setAgent(preferred);
  }, [preferred, agent]);
  useEffect(() => {
    if (settings && iterations === "") setIterations(settings.default_iterations);
  }, [settings, iterations]);

  const noAgents = installed.length === 0;

  async function launch() {
    setLaunching(true);
    setMsg(null);
    try {
      const runId = await launchRun({
        projectId,
        taskAnchor,
        agent,
        maxIterations: Math.max(1, iterations || 1),
      });
      setMsg({ text: "Launched", ok: true });
      onLaunch(runId, agent);
      onLaunched();
    } catch (e) {
      setMsg({ text: String(e), ok: false });
    } finally {
      setLaunching(false);
    }
  }

  return (
    <div className="launch">
      <select
        className="launch__agent"
        value={agent}
        disabled={noAgents}
        onChange={(e) => setAgent(e.target.value)}
        aria-label="Agent"
      >
        {installed.map((k) => (
          <option key={k} value={k}>
            {k}
          </option>
        ))}
      </select>
      <input
        className="launch__iters"
        type="number"
        min={1}
        max={50}
        value={iterations}
        disabled={noAgents}
        onChange={(e) => setIterations(Number(e.target.value))}
        aria-label="Iterations"
        title="Max iterations"
      />
      <button
        className="btn btn--accent launch__go"
        onClick={launch}
        disabled={noAgents || launching || !agent}
        title={noAgents ? "No agent CLI is installed" : undefined}
      >
        {launching ? "Launching…" : "Run"}
      </button>
      {msg && (
        <span className={`msg ${msg.ok ? "msg--ok" : "msg--err"}`}>
          {msg.text}
        </span>
      )}
    </div>
  );
}
