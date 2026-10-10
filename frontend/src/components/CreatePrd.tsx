import { useEffect, useRef, useState } from "react";
import { agentCatalog, appSettings } from "../appData";
import { planCreate, planEditApply, planEditDiscard } from "../commands";
import { renderMarkdown } from "../markdown";
import { Select } from "./Select";
import { ChevronRightIcon } from "./Icon";
import type { AgentStatus, PlanEditProposal } from "../types";

export function CreatePrd({
  projectId,
  onCreated,
}: {
  projectId: string;
  onCreated: () => void;
}) {
  const [agents, setAgents] = useState<AgentStatus[] | null>(null);
  const [agent, setAgent] = useState("");
  const [instruction, setInstruction] = useState("");
  const [proposal, setProposal] = useState<PlanEditProposal | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const active = useRef(false);
  const editId = useRef<string | null>(null);

  useEffect(() => {
    active.current = true;
    Promise.all([agentCatalog(), appSettings()])
      .then(([statuses, settings]) => {
        if (!active.current) return;
        const installed = statuses.filter((status) => status.installed);
        setAgents(installed);
        setAgent(installed.find((status) => status.key === settings.default_agent)?.key ?? installed[0]?.key ?? "");
      })
      .catch((e) => {
        if (active.current) {
          setAgents([]);
          setError(String(e));
        }
      });
    return () => {
      active.current = false;
      if (editId.current) void planEditDiscard(editId.current).catch(() => {});
    };
  }, []);

  async function generate() {
    if (busy || !agent || !instruction.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const draft = await planCreate(projectId, agent, instruction.trim());
      if (!active.current) {
        await planEditDiscard(draft.edit_id);
        return;
      }
      editId.current = draft.edit_id;
      setProposal(draft);
    } catch (e) {
      if (active.current) setError(String(e));
    } finally {
      if (active.current) setBusy(false);
    }
  }

  async function accept() {
    if (!proposal) return;
    setBusy(true);
    setError(null);
    try {
      await planEditApply(proposal.edit_id);
      editId.current = null;
      if (active.current) onCreated();
    } catch (e) {
      if (active.current) setError(String(e));
    } finally {
      if (active.current) setBusy(false);
    }
  }

  async function discard() {
    if (!proposal) return;
    setBusy(true);
    setError(null);
    try {
      await planEditDiscard(proposal.edit_id);
      editId.current = null;
      setProposal(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="prd-doc prd-create" aria-labelledby="prd-create-title" aria-busy={busy}>
      <header className="prd-create__head">
        <span className="prd-create__eyebrow">{proposal ? "Your draft is ready" : "Start a new plan"}</span>
        <h1 id="prd-create-title">{proposal ? "Review your plan" : "What should we build next?"}</h1>
        <p>Turn your brief into a plan you can review and run.</p>
        <ol className="prd-create__steps" aria-label="Planning steps">
          <li aria-current={proposal ? undefined : "step"}><span>1</span>Describe</li>
          <li aria-current={proposal ? "step" : undefined}><span>2</span>Review</li>
          <li><span>3</span>Run tasks</li>
        </ol>
      </header>
      {error && <p className="panel__error" role="alert">{error}</p>}
      {proposal ? (
        <>
          <p className="prd-doc__running-note" title={proposal.path}>Draft by {agents?.find((status) => status.key === proposal.agent)?.display ?? proposal.agent}. Save to create the new plan.</p>
          <div className="prd-doc__body">{renderMarkdown(proposal.proposed)}</div>
          <div className="prd-doc__actions">
            <button type="button" className="btn btn--primary" disabled={busy} onClick={accept}>Save PRD.md</button>
            <button type="button" className="btn" disabled={busy} onClick={discard}>Discard draft</button>
          </div>
        </>
      ) : (
        <form className="prd-doc__instruct" onSubmit={(event) => { event.preventDefault(); void generate(); }}>
          <label htmlFor="prd-create-instruction">Project brief</label>
          <textarea
            id="prd-create-instruction"
            className="prd-doc__instruction"
            placeholder="Describe what you want to build, who it’s for, and what a good result looks like…"
            aria-describedby="prd-create-hint"
            value={instruction}
            onChange={(event) => setInstruction(event.target.value)}
            disabled={busy}
            required
          />
          <p id="prd-create-hint" className="prd-create__hint">Include any requirements or constraints your agent should know.</p>
          <div className="prd-create__controls">
            <label className="field prd-create__agent">
              <span>Draft with</span>
              <Select
                aria-label="Draft with"
                value={agent}
                onChange={setAgent}
                disabled={busy || !agents?.length}
                placeholder={agents === null ? "Loading agents…" : "No installed agent"}
                options={(agents ?? []).map((status) => ({ value: status.key, label: status.display }))}
              />
            </label>
            <button type="submit" className="btn btn--primary" disabled={busy || !agent || !instruction.trim()}>
              {busy ? "Drafting…" : "Draft plan"}
              <ChevronRightIcon size={16} />
            </button>
          </div>
          {busy ? (
            <p className="prd-doc__running-note" role="status">Drafting with {agents?.find((status) => status.key === agent)?.display ?? agent}… You can review it before saving.</p>
          ) : agents?.length === 0 ? (
            <p className="prd-doc__running-note">Install a coding agent and check its availability in Settings.</p>
          ) : (
            <p className="prd-doc__running-note">You’ll review the draft before saving it as PRD.md.</p>
          )}
        </form>
      )}
    </section>
  );
}
