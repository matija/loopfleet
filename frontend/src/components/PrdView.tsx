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
//
// A document can also be archived — moved into the project's `prds/` directory
// and re-keyed, out of the live plan list. That flow is the edit flow's local
// prior art, shrunk to two phases instead of three: `loading` fetches the
// preview (`archive_plan_preview`), then `confirm` shows it — source path,
// destination (noting when it will be created), task/run counts, and an
// editable name field pre-filled from the preview — with Archive disabled
// while the name fails `validArchiveName`. There is no "running" phase because
// `archive_plan` itself is a single file move, not a pass to wait out.

import { useEffect, useMemo, useState } from "react";
import { agentCatalog, appSettings } from "../appData";
import { validArchiveName } from "../archiveName";
import {
  archivePlan,
  archivePlanPreview,
  exportPlanReport,
  planEdit,
  planEditApply,
  planEditDiscard,
  planOverview,
} from "../commands";
import { renderMarkdown } from "../markdown";
import { diffLines } from "../textDiff";
import { NoPlanEmptyState } from "./EmptyState";
import { ExportButton } from "./ExportButton";
import { Patch } from "./RunTimeline";
import type { ArchivePreview, PlanEditProposal, PlanView as Plan } from "../types";

// The edit flow for the one document being edited. `instruct` collects the
// instruction; `running` is the agent pass; `review` shows the returned diff.
type Phase = "instruct" | "running" | "review";

// The archive flow for the one document being archived. `loading` fetches the
// preview; `confirm` shows it alongside the editable name field. No "running"
// phase — see the file header note.
type ArchivePhase = "loading" | "confirm";

export function PrdView({
  projectId,
  onError,
  autoArchivePlanId,
  onAutoArchiveHandled,
}: {
  projectId: string;
  /// The app's toast surface. Used for command failures (a failed export)
  /// and, for archive, the success outcome too — the archived path.
  onError: (message: string) => void;
  /// Set by the command palette's "Archive plan" action to jump straight
  /// into the archive confirmation for that plan once it's loaded, instead
  /// of requiring the Archive button click.
  autoArchivePlanId?: string | null;
  /// Called once `autoArchivePlanId` has been consumed (matched or not), so
  /// the caller can clear it and not re-trigger on the next render.
  onAutoArchiveHandled?: () => void;
}) {
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

  // The one document currently in the archive flow (by plan id), its phase,
  // the fetched preview, the editable name field (pre-filled from the
  // preview's `proposed_file_name`), and any preview/archive error.
  const [archivePlanId, setArchivePlanId] = useState<string | null>(null);
  const [archivePhase, setArchivePhase] = useState<ArchivePhase>("loading");
  const [archivePreview, setArchivePreview] = useState<ArchivePreview | null>(
    null,
  );
  const [archiveNameField, setArchiveNameField] = useState("");
  const [archiveError, setArchiveError] = useState<string | null>(null);
  const [archiveBusy, setArchiveBusy] = useState(false);

  // The configured default agent and whether its CLI is installed, so the edit
  // affordance can name it ("Edit with claude") and disable itself when there is
  // no agent to run. Fetched once; a plain "Edit" label until it resolves.
  const [agent, setAgent] = useState<{ label: string; ready: boolean } | null>(
    null,
  );
  useEffect(() => {
    let cancelled = false;
    Promise.all([appSettings(), agentCatalog()])
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

  function resetArchive() {
    setArchivePlanId(null);
    setArchivePhase("loading");
    setArchivePreview(null);
    setArchiveNameField("");
    setArchiveError(null);
  }

  function load() {
    setPlans(null);
    setError(null);
    resetEdit();
    resetArchive();
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

  // Open the archive flow for `plan`: fetch the preview, then move to confirm.
  // A failed fetch stays in `loading` with the error shown in place of the
  // spinner, so Cancel is still reachable.
  function startArchive(plan: Plan) {
    setArchivePlanId(plan.plan_id);
    setArchivePhase("loading");
    setArchivePreview(null);
    setArchiveNameField("");
    setArchiveError(null);
    archivePlanPreview(plan.plan_id)
      .then((p) => {
        setArchivePreview(p);
        setArchiveNameField(p.proposed_file_name);
        setArchivePhase("confirm");
      })
      .catch((e) => setArchiveError(String(e)));
  }

  // Consume a palette-driven `autoArchivePlanId` once its plan has loaded:
  // jump straight to the archive confirmation, same as clicking that plan's
  // own Archive button. Runs once per id (an unmatched id, e.g. a stale one
  // from a project switch, is dropped silently rather than retried).
  useEffect(() => {
    if (!autoArchivePlanId || !plans) return;
    const plan = plans.find((p) => p.plan_id === autoArchivePlanId);
    if (plan) startArchive(plan);
    onAutoArchiveHandled?.();
  }, [autoArchivePlanId, plans]);

  // Archive: move the file and re-key the plan. The outcome is reported the
  // way the app reports outcomes — the archived path through the toast on
  // success, the command's own message on failure — and either way the plan
  // list reloads, since the flow is over: success drops the doc out of the
  // live list, failure leaves it in place but there's nothing left to confirm.
  async function confirmArchive() {
    if (!archivePlanId) return;
    setArchiveBusy(true);
    setArchiveError(null);
    try {
      const path = await archivePlan(archivePlanId, archiveNameField.trim());
      onError(`Archived to ${path}`);
    } catch (e) {
      onError(String(e));
    } finally {
      setArchiveBusy(false);
      load();
    }
  }

  const archiveNameError =
    archivePhase === "confirm" ? validArchiveName(archiveNameField.trim()) : null;

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
  // The affordances are enabled only when nothing else is being edited or
  // archived (in this doc or another) — one flow at a time — and, for Edit,
  // the default agent is installed.
  const anyFlowOpen = editPlanId !== null || archivePlanId !== null;
  const canStart = agent?.ready === true && !anyFlowOpen && !busy;
  const editDisabledTitle = !agent
    ? undefined
    : !agent.ready
      ? `No installed default agent to edit with — set one in Settings`
      : anyFlowOpen
        ? "Finish the current edit first"
        : undefined;
  const canStartArchive = !anyFlowOpen && !archiveBusy;
  const archiveDisabledTitle = anyFlowOpen
    ? "Finish the current edit first"
    : undefined;

  return (
    <div className="prd">
      {plans.map((plan) => {
        const editing = editPlanId === plan.plan_id;
        const archiving = archivePlanId === plan.plan_id;
        const noChange =
          proposal !== null && proposal.original === proposal.proposed;
        return (
          <article className="prd-doc" key={plan.plan_id}>
            <header className="prd-doc__head">
              <span className="prd-doc__path">{plan.file_path}</span>
              {archiving ? (
                <div className="prd-doc__actions">
                  <button
                    className="btn btn--secondary"
                    onClick={resetArchive}
                    disabled={archiveBusy}
                  >
                    Cancel
                  </button>
                  {archivePhase === "confirm" && (
                    <button
                      className="btn btn--primary"
                      onClick={confirmArchive}
                      disabled={archiveBusy || archiveNameError !== null}
                      title={archiveNameError ?? undefined}
                    >
                      {archiveBusy ? "Archiving…" : "Archive"}
                    </button>
                  )}
                </div>
              ) : editing ? (
                <div className="prd-doc__actions">
                  {phase === "instruct" && (
                    <>
                      <button
                        className="btn btn--secondary"
                        onClick={discard}
                        disabled={busy}
                      >
                        Cancel
                      </button>
                      <button
                        className="btn btn--primary"
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
                        className="btn btn--secondary"
                        onClick={discard}
                        disabled={busy}
                      >
                        Discard
                      </button>
                      <button
                        className="btn btn--primary"
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
                <div className="prd-doc__actions">
                  {/* Exporting is safe to offer at rest but not mid-edit: the
                    * report is built from what is stored, which is not yet what
                    * an unaccepted proposal shows. */}
                  <ExportButton
                    onExport={() => exportPlanReport(plan.plan_id)}
                    onError={onError}
                    title="Save this plan — every task's runs, events, and diffs — as an HTML report"
                  />
                  <button
                    className="btn btn--secondary prd-doc__edit"
                    onClick={() => startEdit(plan)}
                    disabled={!canStart}
                    title={editDisabledTitle}
                  >
                    {editLabel}
                  </button>
                  <button
                    className="btn btn--secondary prd-doc__archive"
                    onClick={() => startArchive(plan)}
                    disabled={!canStartArchive}
                    title={archiveDisabledTitle}
                  >
                    Archive
                  </button>
                </div>
              )}
            </header>

            {archiving && archivePhase === "loading" ? (
              <div className="prd-doc__archive-body">
                {archiveError ? (
                  <p className="panel__error">{archiveError}</p>
                ) : (
                  <p className="prd-doc__archive-loading">
                    Loading archive details…
                  </p>
                )}
              </div>
            ) : archiving && archivePhase === "confirm" && archivePreview ? (
              <div className="prd-doc__archive-body">
                <dl className="prd-doc__archive-facts">
                  <dt>Source</dt>
                  <dd>{archivePreview.file_path}</dd>
                  <dt>Destination</dt>
                  <dd>
                    {archivePreview.destination_dir}
                    {!archivePreview.destination_exists &&
                      " (will be created)"}
                  </dd>
                  <dt>Carries along</dt>
                  <dd>
                    {archivePreview.task_count}{" "}
                    {archivePreview.task_count === 1 ? "task" : "tasks"},{" "}
                    {archivePreview.run_count}{" "}
                    {archivePreview.run_count === 1 ? "run" : "runs"}
                  </dd>
                </dl>
                <label className="prd-doc__archive-name">
                  <span>File name</span>
                  <input
                    type="text"
                    value={archiveNameField}
                    onChange={(e) => setArchiveNameField(e.target.value)}
                    disabled={archiveBusy}
                    spellCheck={false}
                    aria-label="Archive file name"
                  />
                </label>
                {archiveNameError && (
                  <p className="panel__error">{archiveNameError}</p>
                )}
                {archiveError && <p className="panel__error">{archiveError}</p>}
              </div>
            ) : editing && phase === "instruct" ? (
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
