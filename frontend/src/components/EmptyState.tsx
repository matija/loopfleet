// Shared, on-brand empty state: a centered card with a glyph, a title, and
// guidance. Used where a surface has no content yet — chiefly a project with no
// PRD (the plan and PRD panes) — so an unconfigured project reads as an inviting
// next step rather than a muted line of text, and reads the same in both panes.

import type { ReactNode } from "react";

import { DocPlusIcon } from "./Icon";

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
