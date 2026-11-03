import type { ReactNode } from "react";

function EmptyState({
  title,
  children,
}: {
  title: string;
  children?: ReactNode;
}) {
  return (
    <div className="empty-state" role="note">
      <h3 className="empty-state__title">{title}</h3>
      {children && <div className="empty-state__body">{children}</div>}
    </div>
  );
}

export function NoTasksEmptyState({ onOpenPrd }: { onOpenPrd: () => void }) {
  return (
    <EmptyState title="No checklist tasks">
      <pre className="empty-state__example">- [ ] Add a task</pre>
      <button type="button" className="btn btn--primary" onClick={onOpenPrd}>
        Open PRD
      </button>
    </EmptyState>
  );
}

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
