// Task tab: a single task opened from the plan tree, focused. A per-tab
// `CommandBar` at the top hosts the task pill, the agent "Connected"/"missing"
// status pill, and the relocated launch control; the derived `TaskStatus`, task
// text, and review-queue banner sit below. Reuses `LaunchControl` from the plan
// body so launch logic lives in one place.

import { useCallback, useEffect, useState } from "react";
import { agentStatus, getSettings, planOverview } from "../commands";
import type { AgentStatus, PlanView as Plan, Settings } from "../types";
import { CommandBar } from "./CommandBar";
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
  // The pill's agent is the launch default (settings, falling back to the first
  // installed CLI, then the raw default so an all-missing setup still names one).
  const preferred =
    settings && installed.includes(settings.default_agent)
      ? settings.default_agent
      : (installed[0] ?? settings?.default_agent);
  return (
    <div className="task-tab">
      <CommandBar
        task={task.text}
        agent={
          preferred
            ? { name: preferred, connected: installed.includes(preferred) }
            : undefined
        }
      >
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
      </CommandBar>
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
      {task.run_count > 0 && (
        <div className="task-tab__actions">
          <button
            className="task-row__compare"
            onClick={() =>
              onCompare({ planId, taskAnchor, taskText: task.text })
            }
            title="Compare this task's runs and use one"
          >
            {task.run_count} {task.run_count === 1 ? "run" : "runs"} · compare
          </button>
        </div>
      )}
    </div>
  );
}
