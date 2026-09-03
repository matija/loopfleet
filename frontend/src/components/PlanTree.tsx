// Plan tree: the selected connection's plan rendered as the sidebar's object
// list (the DB client's filterable table tree). Tasks group under their plan
// file, each row leading with the derived `TaskStatus` glyph (muted, amber only
// for needs-review) and a right-aligned run-count badge from `plan_overview`.
// Clicking a task opens or focuses its tab.
//
// A group header is a row like any other, not a caption: the chevron toggles
// the group, the title itself opens that plan as a document (the pane's PRD
// view), and a trailing archive button — hover/focus-revealed, the same
// discipline as the connection row's trash icon — starts the existing archive
// flow that moves the file into the project's prds/ directory.

import { useEffect, useState } from "react";
import { planOverview } from "../commands";
import { taskSummary } from "../displayText";
import { useSidebarCollapsed } from "../sidebarCollapse";
import type { PlanView as Plan } from "../types";
import { IconButton } from "./Button";
import { ArchiveIcon, ChevronRightIcon } from "./Icon";
import { STATUS_ICON, STATUS_LABEL } from "./PlanView";

/// What opening a task from the tree needs to push/focus its tab.
export type OpenTask = { planId: string; taskAnchor: string; taskText: string };

export function PlanTree({
  projectId,
  filter,
  activeTaskId,
  nonce,
  onOpenTask,
  onOpenPrd,
  onArchivePlan,
}: {
  projectId: string;
  /// The shared sidebar filter — narrows tasks by text, live.
  filter: string;
  /// Tab id of the currently-open task tab, for the active highlight.
  activeTaskId: string | null;
  /// Bumped to refetch after a launch or accept changes counts/status.
  nonce: number;
  onOpenTask: (task: OpenTask) => void;
  /// Opens the plan as a document — the pane's PRD view for this project.
  onOpenPrd: (planId: string) => void;
  /// Starts the archive flow for a plan (the same one the ⌘K "Archive plan"
  /// action and PrdView's own Archive button run).
  onArchivePlan: (planId: string) => void;
}) {
  const [plans, setPlans] = useState<Plan[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setPlans(null);
    setError(null);
    planOverview(projectId)
      .then(setPlans)
      .catch((e) => setError(String(e)));
  }, [projectId, nonce]);

  if (error) return <div className="plan-tree__note">{error}</div>;
  if (!plans) return <div className="plan-tree__note">Loading tasks…</div>;
  if (plans.length === 0) {
    return <div className="plan-tree__note">No plan file found.</div>;
  }

  const q = filter.trim().toLowerCase();
  const groups = plans
    .map((plan) => ({
      plan,
      tasks: q
        ? plan.tasks.filter((t) => t.text.toLowerCase().includes(q))
        : plan.tasks,
    }))
    .filter((g) => g.tasks.length > 0);

  if (groups.length === 0) {
    return (
      <div className="plan-tree__note">
        {q ? "No tasks match the filter." : "No tasks in this plan."}
      </div>
    );
  }

  return (
    <div className="plan-tree">
      {groups.map(({ plan, tasks }) => (
        <PlanTreeGroup
          key={plan.plan_id}
          plan={plan}
          tasks={tasks}
          activeTaskId={activeTaskId}
          onOpenTask={onOpenTask}
          onOpenPrd={onOpenPrd}
          onArchivePlan={onArchivePlan}
        />
      ))}
    </div>
  );
}

function PlanTreeGroup({
  plan,
  tasks,
  activeTaskId,
  onOpenTask,
  onOpenPrd,
  onArchivePlan,
}: {
  plan: Plan;
  tasks: Plan["tasks"];
  activeTaskId: string | null;
  onOpenTask: (task: OpenTask) => void;
  onOpenPrd: (planId: string) => void;
  onArchivePlan: (planId: string) => void;
}) {
  const [collapsed, toggle] = useSidebarCollapsed(`plan:${plan.plan_id}`);
  const label = plan.title ?? plan.file_path;

  return (
    <div className="plan-tree__group">
      <div className="plan-tree__group-head">
        <button
          type="button"
          className="plan-tree__disclosure"
          aria-expanded={!collapsed}
          aria-label={`${collapsed ? "Expand" : "Collapse"} ${label}`}
          onClick={toggle}
        >
          <ChevronRightIcon size={12} className="disclosure__chevron" />
        </button>
        <button
          type="button"
          className="plan-tree__group-label"
          title={`Open ${label} — ${plan.file_path}`}
          onClick={() => onOpenPrd(plan.plan_id)}
        >
          <span className="plan-tree__group-label-text">{label}</span>
        </button>
        <IconButton
          icon={ArchiveIcon}
          aria-label={`Archive ${label}`}
          title="Archive into prds/"
          className="plan-tree__archive"
          onClick={() => onArchivePlan(plan.plan_id)}
        />
      </div>
      {!collapsed &&
        tasks.map((task) => {
          const id = `task:${plan.plan_id}:${task.anchor}`;
          const StatusIcon = STATUS_ICON[task.status];
          return (
            <button
              key={task.anchor}
              className="tree-item"
              aria-current={id === activeTaskId}
              onClick={() =>
                onOpenTask({
                  planId: plan.plan_id,
                  taskAnchor: task.anchor,
                  taskText: task.text,
                })
              }
            >
              <span
                className={`tree-item__status tree-item__status--${task.status}`}
                role="img"
                aria-label={STATUS_LABEL[task.status]}
                title={STATUS_LABEL[task.status]}
              >
                <StatusIcon size={16} />
              </span>
              <span className="tree-item__text">
                {taskSummary(task.text)}
              </span>
              {task.run_count > 0 && (
                <span
                  className="tree-item__count"
                  title={`${task.run_count} run(s)`}
                >
                  {task.run_count}
                </span>
              )}
            </button>
          );
        })}
    </div>
  );
}
