import { useEffect, useRef, useState } from "react";
import { agentCatalog, appSettings } from "../appData";
import { planCreate, planEditApply, planEditDiscard } from "../commands";
import { renderMarkdown } from "../markdown";
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
      <div className="prd-doc__head">
        <h3 id="prd-create-title">{proposal ? "Review your PRD" : "Create PRD.md"}</h3>
      </div>
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
          <label htmlFor="prd-create-instruction">What should we build next?</label>
          <textarea
            id="prd-create-instruction"
            className="prd-doc__instruction"
            placeholder="Describe the goals, requirements, and constraints…"
            value={instruction}
            onChange={(event) => setInstruction(event.target.value)}
            disabled={busy}
            required
          />
          <div className="prd-create__controls">
            <div className="launch__agents" role="group" aria-label="PRD harness">
              {agents?.map((status) => (
                <button
                  key={status.key}
                  type="button"
                  className={`launch__agent${agent === status.key ? " launch__agent--on" : ""}`}
                  data-label={status.display}
                  aria-pressed={agent === status.key}
                  disabled={busy}
                  onClick={() => setAgent(status.key)}
                >
                  {status.display}
                </button>
              ))}
              {!agents?.length && <span>{agents === null ? "Loading harnesses…" : "No installed harness"}</span>}
            </div>
            <button type="submit" className="btn btn--primary" disabled={busy || !agent || !instruction.trim()}>
              {busy ? "Drafting…" : "Generate PRD"}
            </button>
          </div>
          {busy ? (
            <p className="prd-doc__running-note" role="status">Drafting with {agents?.find((status) => status.key === agent)?.display ?? agent}… You can review it before saving.</p>
          ) : agents?.length === 0 ? (
            <p className="prd-doc__running-note">Install a harness and check its availability in Settings.</p>
          ) : (
            <p className="prd-doc__running-note">Your harness turns this into a plan with runnable tasks.</p>
          )}
        </form>
      )}
    </section>
  );
}
