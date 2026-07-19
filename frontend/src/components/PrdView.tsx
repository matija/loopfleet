// PRD document view: the selected project's plan file(s) rendered as prose,
// alongside the task-list plan view. Where the plan view surfaces the parsed
// checklist with launch controls, this shows the frozen PRD as a human reads it
// — headings, prose, and lists — for reference while supervising runs.
//
// The source is `plan_overview`'s `markdown` field, which the backend reads
// straight from the plan file (`core::overview`, same bytes `plan_document`
// returns), so no extra command is needed. A dependency-free renderer turns it
// into elements (see markdown.tsx).
//
// Each document can be edited in place: "Edit" swaps the rendered prose for the
// raw markdown in a textarea; "Apply" writes it back via `plan_edit` (and reloads
// so the next parse reflects the saved bytes); "Discard" drops the draft and
// returns to the rendered view without touching the file.

import { useEffect, useState } from "react";
import { planEdit, planOverview } from "../commands";
import { renderMarkdown } from "../markdown";
import { NoPlanEmptyState } from "./EmptyState";
import type { PlanView as Plan } from "../types";

export function PrdView({ projectId }: { projectId: string }) {
  const [plans, setPlans] = useState<Plan[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  // The plan currently open for editing (by id) plus its working draft. `null`
  // means no document is being edited — every doc renders as prose.
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  function load() {
    setPlans(null);
    setError(null);
    setEditingId(null);
    setSaveError(null);
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
  }

  useEffect(load, [projectId]);

  function startEdit(plan: Plan) {
    setEditingId(plan.plan_id);
    setDraft(plan.markdown);
    setSaveError(null);
  }

  function discard() {
    setEditingId(null);
    setSaveError(null);
  }

  async function apply(planId: string) {
    setBusy(true);
    setSaveError(null);
    try {
      await planEdit(planId, draft);
      load();
    } catch (e) {
      setSaveError(String(e));
    } finally {
      setBusy(false);
    }
  }

  if (error) return <p className="panel__error">{error}</p>;
  if (!plans) return <p className="plan__loading">Loading document…</p>;
  if (plans.length === 0) return <NoPlanEmptyState />;

  return (
    <div className="prd">
      {plans.map((plan) => {
        const editing = editingId === plan.plan_id;
        return (
          <article className="prd-doc" key={plan.plan_id}>
            <header className="prd-doc__head">
              <span className="prd-doc__path">{plan.file_path}</span>
              {editing ? (
                <div className="prd-doc__actions">
                  <button
                    className="btn"
                    onClick={discard}
                    disabled={busy}
                  >
                    Discard
                  </button>
                  <button
                    className="btn btn--accent"
                    onClick={() => apply(plan.plan_id)}
                    disabled={busy}
                  >
                    {busy ? "Applying…" : "Apply"}
                  </button>
                </div>
              ) : (
                <button
                  className="btn prd-doc__edit"
                  onClick={() => startEdit(plan)}
                  disabled={editingId !== null}
                >
                  Edit
                </button>
              )}
            </header>
            {editing ? (
              <>
                <textarea
                  className="prd-doc__editor"
                  value={draft}
                  onChange={(e) => setDraft(e.target.value)}
                  disabled={busy}
                  spellCheck={false}
                  aria-label="Plan markdown"
                />
                {saveError && <p className="panel__error">{saveError}</p>}
              </>
            ) : (
              <div className="prd-doc__body">{renderMarkdown(plan.markdown)}</div>
            )}
          </article>
        );
      })}
    </div>
  );
}
