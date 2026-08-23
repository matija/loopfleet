// Shared, on-brand empty state: a centered card with a glyph, a title, and
// guidance. Used where a surface has no content yet — chiefly a project with no
// PRD (the plan and PRD panes) — so an unconfigured project reads as an inviting
// next step rather than a muted line of text, and reads the same in both panes.

import type { ReactNode } from "react";

import { ChecklistIcon, DocPlusIcon } from "./Icon";

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

// The "no PRD" empty state, shared by the plan and PRD panes so a project
// without a plan file tells the same story in both.
export function NoPlanEmptyState() {
  return (
    <EmptyState icon={<DocPlusIcon />} title="No PRD yet">
      <p>
        This project has no plan to run against. Add a <code>PRD.md</code> at the
        repo root, or a <code>plans/</code> folder of <code>.md</code> files.
      </p>
      <p>Each checklist item becomes a task you can launch an agent on.</p>
    </EmptyState>
  );
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
        <button type="button" className="btn btn--accent" onClick={onAddProject}>
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
        <button type="button" className="btn btn--accent" onClick={onLaunch}>
          Launch a run
        </button>
      }
    />
  );
}
