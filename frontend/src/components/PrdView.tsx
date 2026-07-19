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
// Each document can be edited by the default AI agent — never by hand (manual
// markdown editing is a non-goal) and never as a silent write. The affordance
// names the agent that will run ("Edit with claude"), reading it from
// `agent_status`, and is a disabled state when no default agent is installed.
// The flow: name the agent → take a free-text instruction → run one pass in an
// isolated worktree (`plan_edit`, the "running" state) → the returned edit
// renders as a reviewable diff of original vs proposed (the design language, no
// raw markdown editor) → Accept writes the file (`plan_edit_apply`) or Discard
// drops the worktree untouched (`plan_edit_discard`). A failed pass or an edit
// that changed nothing reads as an intentional state, not a dead end.

import { useEffect, useMemo, useState } from "react";
import {
  agentStatus,
  getSettings,
  planEdit,
  planEditApply,
  planEditDiscard,
  planOverview,
} from "../commands";
import { renderMarkdown } from "../markdown";
import { diffLines } from "../textDiff";
import { NoPlanEmptyState } from "./EmptyState";
import { Patch } from "./RunTimeline";
import type { PlanEditProposal, PlanView as Plan } from "../types";

// The edit flow for the one document being edited. `instruct` collects the
// instruction; `running` is the agent pass; `review` shows the returned diff.
type Phase = "instruct" | "running" | "review";

export function PrdView({ projectId }: { projectId: string }) {
  const [plans, setPlans] = useState<Plan[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  // The one document currently in the edit flow (by plan id), plus that flow's
  // phase, the instruction being composed, the returned proposal once it lands,
  // and any pass error. `null` plan id means every doc renders as prose.
  const [editPlanId, setEditPlanId] = useState<string | null>(null);
  const [phase, setPhase] = useState<Phase>("instruct");
  const [instruction, setInstruction] = useState("");
  const [proposal, setProposal] = useState<PlanEditProposal | null>(null);
  const [editError, setEditError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // The configured default agent and whether its CLI is installed, so the edit
  // affordance can name it ("Edit with claude") and disable itself when there is
  // no agent to run. Fetched once; a plain "Edit" label until it resolves.
  const [agent, setAgent] = useState<{ label: string; ready: boolean } | null>(
    null,
  );
  useEffect(() => {
    let cancelled = false;
    Promise.all([getSettings(), agentStatus()])
      .then(([settings, statuses]) => {
        if (cancelled) return;
        const s = statuses.find((a) => a.key === settings.default_agent);
        setAgent({
          label: s?.display ?? settings.default_agent,
          ready: s?.installed ?? false,
        });
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  function resetEdit() {
    setEditPlanId(null);
    setPhase("instruct");
    setInstruction("");
    setProposal(null);
    setEditError(null);
  }

  function load() {
    setPlans(null);
    setError(null);
    resetEdit();
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
    setEditPlanId(plan.plan_id);
    setPhase("instruct");
    setInstruction("");
    setProposal(null);
    setEditError(null);
  }

  // Run one edit pass. On success the returned edit moves to review; on failure
  // the message shows and the instruction stays so the user can adjust and retry.
  async function run(planId: string) {
    setBusy(true);
    setPhase("running");
    setEditError(null);
    try {
      const p = await planEdit(planId, instruction.trim());
      setProposal(p);
      setPhase("review");
    } catch (e) {
      setEditError(String(e));
      setPhase("instruct");
    } finally {
      setBusy(false);
    }
  }

  // Accept: write the proposed markdown and reload so the prose reflects it.
  async function accept() {
    if (!proposal) return;
    setBusy(true);
    setEditError(null);
    try {
      await planEditApply(proposal.edit_id);
      load();
    } catch (e) {
      setEditError(String(e));
    } finally {
      setBusy(false);
    }
  }

  // Discard: drop the scratch worktree (if a proposal exists) and return to the
  // document unchanged. Also the "cancel" for the instruction step.
  async function discard() {
    setBusy(true);
    try {
      if (proposal) await planEditDiscard(proposal.edit_id);
    } catch {
      // A failed cleanup shouldn't trap the user in the edit flow; the worktree
      // is app-managed scratch and gets pruned on next launch anyway.
    } finally {
      setBusy(false);
      resetEdit();
    }
  }

  // The returned edit as a diff, computed only in review.
  const review = useMemo(
    () =>
      proposal ? diffLines(proposal.original, proposal.proposed) : null,
    [proposal],
  );

  if (error) return <p className="panel__error">{error}</p>;
  if (!plans) return <p className="plan__loading">Loading document…</p>;
  if (plans.length === 0) return <NoPlanEmptyState />;

  const editLabel = agent ? `Edit with ${agent.label}` : "Edit";
  // The affordance is enabled only when nothing else is being edited and the
  // default agent is installed.
  const canStart = agent?.ready === true && editPlanId === null && !busy;
  const editDisabledTitle = !agent
    ? undefined
    : !agent.ready
      ? `No installed default agent to edit with — set one in Settings`
      : editPlanId !== null
        ? "Finish the current edit first"
        : undefined;

  return (
    <div className="prd">
      {plans.map((plan) => {
        const editing = editPlanId === plan.plan_id;
        const noChange =
          proposal !== null && proposal.original === proposal.proposed;
        return (
          <article className="prd-doc" key={plan.plan_id}>
            <header className="prd-doc__head">
              <span className="prd-doc__path">{plan.file_path}</span>
              {editing ? (
                <div className="prd-doc__actions">
                  {phase === "instruct" && (
                    <>
                      <button
                        className="btn"
                        onClick={discard}
                        disabled={busy}
                      >
                        Cancel
                      </button>
                      <button
                        className="btn btn--accent"
                        onClick={() => run(plan.plan_id)}
                        disabled={busy || instruction.trim() === ""}
                        title={
                          instruction.trim() === ""
                            ? "Describe the edit first"
                            : undefined
                        }
                      >
                        {agent ? `Run ${agent.label}` : "Run"}
                      </button>
                    </>
                  )}
                  {phase === "running" && (
                    <span className="prd-doc__running" role="status">
                      <span className="prd-doc__spinner" aria-hidden="true" />
                      {agent ? `${agent.label} is editing…` : "Editing…"}
                    </span>
                  )}
                  {phase === "review" && (
                    <>
                      <button
                        className="btn"
                        onClick={discard}
                        disabled={busy}
                      >
                        Discard
                      </button>
                      <button
                        className="btn btn--accent"
                        onClick={accept}
                        disabled={busy || noChange}
                        title={
                          noChange ? "Nothing to write — the edit is empty" : undefined
                        }
                      >
                        {busy ? "Applying…" : "Accept"}
                      </button>
                    </>
                  )}
                </div>
              ) : (
                <button
                  className="btn prd-doc__edit"
                  onClick={() => startEdit(plan)}
                  disabled={!canStart}
                  title={editDisabledTitle}
                >
                  {editLabel}
                </button>
              )}
            </header>

            {editing && phase === "instruct" ? (
              <div className="prd-doc__instruct">
                <textarea
                  className="prd-doc__instruction"
                  value={instruction}
                  onChange={(e) => setInstruction(e.target.value)}
                  disabled={busy}
                  spellCheck={false}
                  placeholder={`Describe the edit for ${
                    agent ? agent.label : "the agent"
                  } to make — e.g. "add an acceptance-criteria section to each task".`}
                  aria-label="Edit instruction"
                />
                {editError && <p className="panel__error">{editError}</p>}
              </div>
            ) : editing && phase === "running" ? (
              <div className="prd-doc__review prd-doc__review--running">
                <p className="prd-doc__running-note">
                  Running one pass in an isolated worktree. The document is
                  untouched until you accept.
                </p>
              </div>
            ) : editing && phase === "review" && proposal ? (
              <div className="prd-doc__review">
                {review && review.insertions + review.deletions > 0 ? (
                  <>
                    <div className="prd-doc__review-stat">
                      {review.insertions > 0 && (
                        <span className="diff__ins">+{review.insertions}</span>
                      )}
                      {review.deletions > 0 && (
                        <span className="diff__del">−{review.deletions}</span>
                      )}
                      <span className="prd-doc__review-count">
                        {review.insertions + review.deletions}{" "}
                        {review.insertions + review.deletions === 1
                          ? "line"
                          : "lines"}{" "}
                        changed
                      </span>
                    </div>
                    <Patch text={review.patch} />
                  </>
                ) : (
                  <p className="timeline__no-diff">
                    {proposal.agent} made no changes. Discard to return to the
                    document.
                  </p>
                )}
                {editError && <p className="panel__error">{editError}</p>}
              </div>
            ) : (
              <div className="prd-doc__body">{renderMarkdown(plan.markdown)}</div>
            )}
          </article>
        );
      })}
    </div>
  );
}
