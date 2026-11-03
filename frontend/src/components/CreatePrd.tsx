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
  const [pending, setPending] = useState<"save" | "discard" | null>(null);
  const [activity, setActivity] = useState<PrdActivityUpdate | null>(null);
  const [error, setError] = useState<string | null>(null);
  const active = useRef(false);
  const submitting = useRef(false);
  const reviewHeading = useRef<HTMLHeadingElement>(null);
  const brief = useRef<HTMLTextAreaElement>(null);
  const restoreBriefFocus = useRef(false);
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

  useEffect(() => {
    if (!proposal && !busy && restoreBriefFocus.current) {
      brief.current?.focus();
      restoreBriefFocus.current = false;
    }
  }, [proposal, busy]);

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
    setPending("save");
    setError(null);
    try {
      await planEditApply(proposal.edit_id);
      editId.current = null;
      if (active.current) onCreated();
    } catch (e) {
      if (active.current) setError(String(e));
    } finally {
      submitting.current = false;
      if (active.current) {
        setBusy(false);
        setPending(null);
      }
    }
  }

  async function discard() {
    if (submitting.current || !proposal) return;
    submitting.current = true;
    setBusy(true);
    setPending("discard");
    setError(null);
    try {
      await planEditDiscard(proposal.edit_id);
      editId.current = null;
      if (active.current) {
        restoreBriefFocus.current = true;
        setProposal(null);
      }
    } catch (e) {
      if (active.current) setError(String(e));
    } finally {
      submitting.current = false;
      if (active.current) {
        setBusy(false);
        setPending(null);
      }
    }
  }

  return (
    <section className="prd-doc prd-create" aria-labelledby="prd-create-title">
      <header className="prd-create__head">
        <h1 id="prd-create-title" ref={reviewHeading} tabIndex={-1}>{proposal ? "Review plan" : busy ? "Drafting plan…" : "New plan"}</h1>
      </header>
      {error && <p className="panel__error" role="alert">{error}</p>}
      {proposal ? (
        <>
          <p className="prd-create__destination"><code>{proposal.path}</code></p>
          <div className="prd-doc__body">{renderMarkdown(proposal.proposed)}</div>
          <div className="prd-doc__actions prd-create__actions" aria-busy={pending !== null}>
            <button type="button" className="btn btn--primary" disabled={busy} onClick={accept}>{pending === "save" ? "Saving…" : "Save plan"}</button>
            <button type="button" className="btn" disabled={busy} onClick={discard}>{pending === "discard" ? "Discarding…" : "Discard draft"}</button>
          </div>
        </>
      ) : (
        <form className="prd-doc__instruct" onSubmit={(event) => { event.preventDefault(); void generate(); }}>
          {busy && <PrdActivity title={`Drafting with ${agents?.find((status) => status.key === agent)?.display ?? agent}`} activity={activity} />}
          <label htmlFor="prd-create-instruction">Brief</label>
          <textarea
            ref={brief}
            id="prd-create-instruction"
            className="prd-doc__instruction"
            placeholder="What do you want to build?"
            value={instruction}
            onChange={(event) => setInstruction(event.target.value)}
            disabled={busy}
            required
          />
          <div className="prd-create__controls">
            <label className="field prd-create__agent">
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
          ) : null}
        </form>
      )}
    </section>
  );
}
