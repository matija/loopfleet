// Plan tree: the selected connection's plan rendered as the sidebar's object
// list (the DB client's filterable table tree). Tasks group under their plan
// file, each row carrying the derived `TaskStatus` (a colored dot) and a
// right-aligned run-count badge from `plan_overview`. Completed-unaccepted tasks
// are surfaced loudly (a warn accent) — the review queue. Clicking a task opens
// or focuses its tab.

import { useEffect, useState } from "react";
import { planOverview } from "../commands";
import type { PlanView as Plan } from "../types";

/// What opening a task from the tree needs to push/focus its tab.
export type OpenTask = { planId: string; taskAnchor: string; taskText: string };

export function PlanTree({
  projectId,
  filter,
  activeTaskId,
  nonce,
  onOpenTask,
}: {
  projectId: string;
  /// The shared sidebar filter — narrows tasks by text, live.
  filter: string;
  /// Tab id of the currently-open task tab, for the active highlight.
  activeTaskId: string | null;
  /// Bumped to refetch after a launch or accept changes counts/status.
  nonce: number;
  onOpenTask: (task: OpenTask) => void;
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
    return <div className="plan-tree__note">No tasks match the filter.</div>;
  }

  return (
    <div className="plan-tree">
      {groups.map(({ plan, tasks }) => (
        <div key={plan.plan_id} className="plan-tree__group">
          <div className="plan-tree__group-label">
            {plan.title ?? plan.file_path}
          </div>
          {tasks.map((task) => {
            const id = `task:${plan.plan_id}:${task.anchor}`;
            const review = task.status === "completed-unaccepted";
            return (
              <button
                key={task.anchor}
                className={`tree-item${review ? " tree-item--review" : ""}`}
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
                  className={`tree-item__dot tree-item__dot--${task.status}`}
                  title={task.status}
                />
                <span className="tree-item__text">{task.text}</span>
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
      ))}
    </div>
  );
}
