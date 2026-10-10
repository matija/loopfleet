import { useEffect, useRef, useState } from "react";
import { agentCatalog, appSettings } from "../appData";
import { planCreate, planEditApply, planEditDiscard } from "../commands";
import { renderMarkdown } from "../markdown";
import { Select } from "./Select";
import { ChevronRightIcon } from "./Icon";
import { PrdActivity, type PrdActivityUpdate } from "./PrdActivity";
import type { AgentStatus, PlanEditProposal } from "../types";

export function CreatePrd({
  projectId,
  onCreated,
  onOpenAgentSettings,
}: {
  projectId: string;
  onCreated: () => void;
  onOpenAgentSettings?: () => void;
}) {
  const [agents, setAgents] = useState<AgentStatus[] | null>(null);
  const [discoveryError, setDiscoveryError] = useState<string | null>(null);
  const [discoveryAttempt, setDiscoveryAttempt] = useState(0);
  const [agent, setAgent] = useState("");
  const [instruction, setInstruction] = useState("");
  const [proposal, setProposal] = useState<PlanEditProposal | null>(null);
  const [busy, setBusy] = useState(false);
  const [activity, setActivity] = useState<PrdActivityUpdate | null>(null);
  const [error, setError] = useState<string | null>(null);
  const active = useRef(false);
  const submitting = useRef(false);
  const reviewHeading = useRef<HTMLHeadingElement>(null);
  const selectedAgent = agents?.find((status) => status.key === agent);
  const editId = useRef<string | null>(null);

  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
      if (editId.current) void planEditDiscard(editId.current).catch(() => {});
    };
  }, []);

  useEffect(() => {
    let obsolete = false;
    setAgents(null);
    setDiscoveryError(null);
    Promise.all([agentCatalog(), appSettings()])
      .then(([statuses, settings]) => {
        if (obsolete) return;
        const installed = statuses.filter((status) => status.installed);
        setAgents(installed);
        setAgent(installed.find((status) => status.key === settings.default_agent)?.key ?? installed[0]?.key ?? "");
      })
      .catch((e) => {
        if (!obsolete) setDiscoveryError(String(e));
      });
    return () => { obsolete = true; };
  }, [discoveryAttempt]);

  useEffect(() => {
    if (proposal) reviewHeading.current?.focus();
  }, [proposal]);

  async function generate() {
    if (submitting.current || proposal || !selectedAgent || !instruction.trim()) return;
    submitting.current = true;
    setBusy(true);
    setActivity(null);
    setError(null);
    try {
      const draft = await planCreate(projectId, agent, instruction.trim(), (message) => {
        if (active.current) setActivity({ message, at: Date.now() });
      });
      if (!active.current) {
        await planEditDiscard(draft.edit_id);
        return;
      }
      editId.current = draft.edit_id;
      setProposal(draft);
    } catch (e) {
      if (active.current) setError(String(e));
    } finally {
      submitting.current = false;
      if (active.current) setBusy(false);
    }
  }

  async function accept() {
    if (submitting.current || !proposal) return;
    submitting.current = true;
    setBusy(true);
    setError(null);
    try {
      await planEditApply(proposal.edit_id);
      editId.current = null;
      if (active.current) onCreated();
    } catch (e) {
      if (active.current) setError(String(e));
    } finally {
      submitting.current = false;
      if (active.current) setBusy(false);
    }
  }

  async function discard() {
    if (submitting.current || !proposal) return;
    submitting.current = true;
    setBusy(true);
    setError(null);
    try {
      await planEditDiscard(proposal.edit_id);
      editId.current = null;
      if (active.current) setProposal(null);
    } catch (e) {
      if (active.current) setError(String(e));
    } finally {
      submitting.current = false;
      if (active.current) setBusy(false);
    }
  }

  return (
    <section className="prd-doc prd-create" aria-labelledby="prd-create-title">
      <header className="prd-create__head">
        <h1 id="prd-create-title" ref={reviewHeading} tabIndex={-1}>{proposal ? "Review your plan" : busy ? "Drafting your plan" : "What should we build next?"}</h1>
        <p>Turn your brief into a plan you can review and run.</p>
        <ol className="prd-create__steps" aria-label="Planning steps">
          <li aria-current={proposal ? undefined : "step"}><span>1</span>Describe</li>
          <li aria-current={proposal ? "step" : undefined}><span>2</span>Review</li>
          <li><span>3</span>Run tasks</li>
        </ol>
      </header>
      <p role="status" aria-live="polite">{busy && !proposal ? `Drafting with ${selectedAgent?.display ?? agent}` : ""}</p>
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
          {busy && <PrdActivity title={`Drafting with ${agents?.find((status) => status.key === agent)?.display ?? agent}`} activity={activity} />}
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
                placeholder={discoveryError !== null ? "Agent discovery failed" : agents === null ? "Loading agents…" : "No installed agent"}
                options={(agents ?? []).map((status) => ({ value: status.key, label: status.display }))}
              />
            </label>
            <button type="submit" className="btn btn--primary" disabled={busy || !selectedAgent || !instruction.trim()}>
              {busy ? "Drafting…" : "Draft plan"}
              <ChevronRightIcon size={16} />
            </button>
          </div>
          {discoveryError !== null ? (
            <div role="alert">
              <p className="panel__error">Could not load agents: {discoveryError}</p>
              <button type="button" className="btn" onClick={() => setDiscoveryAttempt((value) => value + 1)}>Retry</button>
            </div>
          ) : agents?.length === 0 ? (
            <p className="prd-doc__running-note">No installed agent. Check availability in Agents settings. {onOpenAgentSettings && <button type="button" className="btn" onClick={onOpenAgentSettings}>Open agent settings</button>}</p>
          ) : (
            <p className="prd-doc__running-note">You’ll review the draft before saving it as PRD.md.</p>
          )}
        </form>
      )}
    </section>
  );
}
