// Agent CLI availability + version-drift chips. One chip per v1 agent: green +
// version when installed, a yellow "tested <v>" drift warning when the detected
// version differs from the one the adapter was tested against, red + reason when
// missing (a run with a missing CLI is refused, so this is a launch guardrail).

import { useEffect, useState } from "react";
import { agentStatus } from "../commands";
import type { AgentStatus } from "../types";

export function AgentStatusPanel() {
  // `loaded` distinguishes "fetching" from "fetched an empty set" — without it,
  // an in-flight load renders an empty chip row that reads as "no agents".
  const [agents, setAgents] = useState<AgentStatus[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    agentStatus()
      .then((a) => {
        if (cancelled) return;
        setAgents(a);
        setLoaded(true);
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e));
        setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <section className="panel">
      <div className="panel__head">
        <h3>Agents</h3>
      </div>
      {error ? (
        <p className="panel__error">{error}</p>
      ) : !loaded ? (
        <p className="panel__loading">Detecting agent CLIs…</p>
      ) : agents.length === 0 ? (
        <p className="panel__empty">
          No agent CLIs detected. Install <code>claude</code>, <code>pi</code>, or{" "}
          <code>cursor</code> to launch runs.
        </p>
      ) : (
        <div className="agent-chips">
          {agents.map((a) => (
            <AgentChip key={a.key} agent={a} />
          ))}
        </div>
      )}
    </section>
  );
}

function AgentChip({ agent }: { agent: AgentStatus }) {
  const drift = agent.installed && agent.version_matches === false;
  const state = !agent.installed ? "off" : drift ? "drift" : "on";
  return (
    <div className={`agent-chip agent-chip--${state}`}>
      <span className="agent-chip__dot" />
      <span className="agent-chip__name">{agent.display}</span>
      <span className="agent-chip__ver">
        {agent.installed
          ? (agent.version ?? "installed")
          : (agent.detail ?? "not installed")}
      </span>
      {drift && (
        <span
          className="agent-chip__warn"
          title={`Adapter tested against ${agent.tested_version}; you have ${agent.version}`}
        >
          tested {agent.tested_version}
        </span>
      )}
    </div>
  );
}
