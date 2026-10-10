import type { ReactNode } from "react";
import { CreatePrd } from "./CreatePrd";

import { ChecklistIcon } from "./Icon";

export function EmptyState({
  icon,
  title,
  children,
}: {
  icon?: ReactNode;
  title: string;
  children?: ReactNode;
}) {
  return (
    <div className="empty-state" role="note">
      {icon && (
        <div className="empty-state__icon" aria-hidden="true">
          {icon}
        </div>
      )}
      <h3 className="empty-state__title">{title}</h3>
      {children && <div className="empty-state__body">{children}</div>}
    </div>
  );
}

export function NoPlanEmptyState(props: { projectId: string; onCreated: () => void }) {
  return <CreatePrd {...props} />;
}

// The "no tasks" empty state: the plan file parsed fine, it just has nothing
// runnable in it. The example line is deliberately literal — the parser only
// picks up `- [ ]` checklist items, and a plan written as prose or as `*`
// bullets is the likeliest reason a real plan lands here.
export function NoTasksEmptyState() {
  return (
    <EmptyState icon={<ChecklistIcon size={26} />} title="No tasks in this plan">
      <p>
        Loopfleet runs against <code>- [ ]</code> checklist items — other lines
        in the file are context, not tasks.
      </p>
      <pre className="empty-state__example">- [ ] Add a health check endpoint</pre>
      <p>Add a line like that to this plan file to launch a run against it.</p>
    </EmptyState>
  );
}

// A lighter-weight empty state for surfaces with more headroom than a card
// suits — no border, no icon badge, just a sentence, a supporting line, and
// one way forward. Used where nothing has happened yet at all (no project,
// no run), rather than where content is merely missing from an otherwise
// configured project.
export function PromptEmptyState({
  title,
  subtitle,
  action,
}: {
  title: string;
  subtitle?: string;
  action?: ReactNode;
}) {
  return (
    <div className="empty-prompt" role="note">
      <p className="empty-prompt__title">{title}</p>
      {subtitle && <p className="empty-prompt__subtitle">{subtitle}</p>}
      {action && <div className="empty-prompt__action">{action}</div>}
    </div>
  );
}

// The no-project state: shown when the machine has no projects to select yet.
export function NoProjectEmptyState({
  onAddProject,
}: {
  onAddProject: () => void;
}) {
  return (
    <PromptEmptyState
      title="Add a project to get started"
      subtitle="Point Loopfleet at a git repo with a plan to launch runs against."
      action={
        <button type="button" className="btn btn--primary" onClick={onAddProject}>
          Add project
        </button>
      }
    />
  );
}

// The no-run state: shown when a project has tasks but no run has been
// launched against any of them yet.
export function NoRunEmptyState({ onLaunch }: { onLaunch: () => void }) {
  return (
    <PromptEmptyState
      title="Launch a run to see it here"
      subtitle="Pick a task from the plan and start an agent against it."
      action={
        <button type="button" className="btn btn--primary" onClick={onLaunch}>
          Launch a run
        </button>
      }
    />
  );
}
