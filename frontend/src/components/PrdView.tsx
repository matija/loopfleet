// PRD document view: the selected project's plan file(s) rendered as prose,
// alongside the task-list plan view. Where the plan view surfaces the parsed
// checklist with launch controls, this shows the frozen PRD as a human reads it
// — headings, prose, and lists — for reference while supervising runs.
//
// The source is `plan_overview`'s `markdown` field, which the backend reads
// straight from the plan file (`core::overview`, same bytes `plan_document`
// returns), so no extra command is needed. A dependency-free renderer turns it
// into elements (see markdown.tsx).

import { useEffect, useState } from "react";
import { planOverview } from "../commands";
import { renderMarkdown } from "../markdown";
import { NoPlanEmptyState } from "./EmptyState";
import type { PlanView as Plan } from "../types";

export function PrdView({ projectId }: { projectId: string }) {
  const [plans, setPlans] = useState<Plan[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setPlans(null);
    setError(null);
    let cancelled = false;
    planOverview(projectId)
      .then((ps) => {
        if (!cancelled) setPlans(ps);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [projectId]);

  if (error) return <p className="panel__error">{error}</p>;
  if (!plans) return <p className="plan__loading">Loading document…</p>;
  if (plans.length === 0) return <NoPlanEmptyState />;

  return (
    <div className="prd">
      {plans.map((plan) => (
        <article className="prd-doc" key={plan.plan_id}>
          <header className="prd-doc__head">
            <span className="prd-doc__path">{plan.file_path}</span>
          </header>
          <div className="prd-doc__body">{renderMarkdown(plan.markdown)}</div>
        </article>
      ))}
    </div>
  );
}
