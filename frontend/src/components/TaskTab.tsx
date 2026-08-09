// Task tab: a single task opened from the plan tree. A compact toolbar keeps its
// status, plan, and launch controls together; task text and review state follow.
// Reuses `LaunchControl` from the plan body so launch logic lives in one place.

import { useCallback, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { agentStatus, planOverview } from "../commands";
import { normalizeDisplayText } from "../displayText";
import type { AgentStatus, PlanView as Plan } from "../types";
import { AlertIcon } from "./Icon";
import {
  LaunchControl,
  STATUS_ICON,
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
  toolbarActions,
}: {
  projectId: string;
  planId: string;
  taskAnchor: string;
  /// Bumped by App after a launch/accept so the tab reflects fresh status/counts.
  nonce: number;
  onLaunch: (run: LaunchedRun) => void;
  onCompare: (target: CompareTarget) => void;
  onLaunched: () => void;
  /// The toolbar's action-slot DOM node (App's Toolbar ref). Run and Compare
  /// portal there instead of rendering inline.
  toolbarActions: HTMLElement | null;
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

  useEffect(() => {
    setPlans(null);
    setError(null);
    reload();
  }, [reload, nonce]);

  useEffect(() => {
    agentStatus()
      .then(setAgents)
      .catch(() => {})
      .finally(() => setAgentsLoading(false));
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
  const StatusIcon = STATUS_ICON[task.status];
  return (
    <div className="task-tab">
      <div className="task-tab__toolbar">
        <div className="task-tab__meta">
          <span className={`task-status task-status--${task.status}`}>
            <StatusIcon size={16} className="task-status__icon" />
            <span className="task-status__label">{STATUS_LABEL[task.status]}</span>
          </span>
          <span className="task-tab__plan">{plan.title ?? plan.file_path}</span>
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
          actionsPortal={toolbarActions}
        />
      </div>
      <h3 className="task-tab__text">{normalizeDisplayText(task.text)}</h3>
      {review && (
        <div className="review-banner" role="status">
          <AlertIcon size={16} className="review-banner__icon" />
          This run is awaiting review — compare its diff and use one, or keep
          iterating.
        </div>
      )}
      {task.run_count > 0 &&
        toolbarActions &&
        createPortal(
          <button
            className="task-row__compare"
            onClick={() =>
              onCompare({ planId, taskAnchor, taskText: task.text })
            }
            title="Compare this task's runs and use one"
          >
            {task.run_count} {task.run_count === 1 ? "run" : "runs"} · compare
          </button>,
          toolbarActions,
        )}
    </div>
  );
}
