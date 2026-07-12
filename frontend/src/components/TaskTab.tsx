// Task tab: a single task opened from the plan tree, focused. Shows the derived
// `TaskStatus`, the task text, the review-queue banner when completed-unaccepted,
// and the launch control (relocation of the launch control into a command bar is
// the next task). Reuses `LaunchControl` from the plan body so launch logic lives
// in one place.

import { useCallback, useEffect, useState } from "react";
import { agentStatus, getSettings, planOverview } from "../commands";
import type { AgentStatus, PlanView as Plan, Settings } from "../types";
import {
  LaunchControl,
  STATUS_LABEL,
  type CompareTarget,
  type LaunchedRun,
} from "./PlanView";

export function TaskTab({
  projectId,
  planId,
  taskAnchor,
  nonce,
  onLaunch,
  onCompare,
  onLaunched,
}: {
  projectId: string;
  planId: string;
  taskAnchor: string;
  /// Bumped by App after a launch/accept so the tab reflects fresh status/counts.
  nonce: number;
  onLaunch: (run: LaunchedRun) => void;
  onCompare: (target: CompareTarget) => void;
  onLaunched: () => void;
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

  useEffect(() => {
    setPlans(null);
    setError(null);
    reload();
  }, [reload, nonce]);

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
  if (!plans) return <p className="plan__loading">Loading task…</p>;
  const plan = plans.find((p) => p.plan_id === planId);
  const task = plan?.tasks.find((t) => t.anchor === taskAnchor);
  if (!plan || !task) {
    return <p className="plan__empty">This task is no longer in the plan.</p>;
  }

  const review = task.status === "completed-unaccepted";
  return (
    <div className="task-tab">
      <div className="task-tab__meta">
        <span className={`task-badge task-badge--${task.status}`}>
          {STATUS_LABEL[task.status]}
        </span>
        <span className="task-tab__plan">{plan.title ?? plan.file_path}</span>
      </div>
      {review && (
        <div className="review-banner" role="status">
          This run is awaiting review — compare its diff and use one, or keep
          iterating.
        </div>
      )}
      <p className="task-tab__text">{task.text}</p>
      <div className="task-tab__actions">
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
        {task.run_count > 0 && (
          <button
            className="task-row__compare"
            onClick={() =>
              onCompare({ planId, taskAnchor, taskText: task.text })
            }
            title="Compare this task's runs and use one"
          >
            {task.run_count} {task.run_count === 1 ? "run" : "runs"} · compare
          </button>
        )}
      </div>
    </div>
  );
}
