// Agent CLI availability + version-drift chips. One chip per v1 agent: green +
// version when installed, a yellow "tested <v>" drift warning when the detected
// version differs from the one the adapter was tested against, red + reason when
// missing (a run with a missing CLI is refused, so this is a launch guardrail).
//
// Each installed agent's chip also carries its limit headroom beside the
// version, so the same glance that answers "can I launch this agent?" answers
// "will it get anywhere?". The wording and bucketing are `usage.ts`'s; this
// file only fetches, ticks, and renders.

import { useEffect, useState } from "react";
import { agentStatus, agentUsage } from "../commands";
import { onAgentUsage } from "../events";
import type { AgentStatus, UsageSnapshot } from "../types";
import { usageIndicator } from "../usage";

/// How often the headroom text is re-resolved. Nothing pushes an event when a
/// snapshot merely *ages* past the staleness cutoff, so the chip re-reads the
/// clock on its own; a minute is finer than the 15-minute cutoff it guards and
/// coarse enough to be invisible.
const TICK_MS = 60 * 1_000;

export function AgentStatusPanel() {
  // `loaded` distinguishes "fetching" from "fetched an empty set" — without it,
  // an in-flight load renders an empty chip row that reads as "no agents".
  const [agents, setAgents] = useState<AgentStatus[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [usage, setUsage] = useState<Record<string, UsageSnapshot>>({});
  const [now, setNow] = useState(() => Date.now());

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

  // Headroom is a nicety beside the availability answer: a failed probe leaves
  // the chips reading "usage unknown" rather than taking the whole panel down.
  useEffect(() => {
    let cancelled = false;
    agentUsage()
      .then((snapshots) => {
        if (cancelled) return;
        setUsage(byAgent(snapshots));
      })
      .catch(() => {});
    const unlisten = onAgentUsage((snapshot) => {
      setUsage((prior) => ({ ...prior, [snapshot.agent_key]: snapshot }));
    });
    return () => {
      cancelled = true;
      unlisten.then((stop) => stop());
    };
  }, []);

  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), TICK_MS);
    return () => clearInterval(id);
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
            <AgentChip
              key={a.key}
              agent={a}
              usage={usage[a.key] ?? null}
              now={now}
            />
          ))}
        </div>
      )}
    </section>
  );
}

/// Latest snapshot per agent key. The backend sends one per agent already; this
/// keeps the lookup a map rather than a scan on every render.
function byAgent(snapshots: UsageSnapshot[]): Record<string, UsageSnapshot> {
  return Object.fromEntries(snapshots.map((s) => [s.agent_key, s]));
}

function AgentChip({
  agent,
  usage,
  now,
}: {
  agent: AgentStatus;
  usage: UsageSnapshot | null;
  now: number;
}) {
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
      {/* A missing CLI's headroom is beside the point — the launch is refused
          on availability first. */}
      {agent.installed && <Headroom usage={usage} now={now} />}
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

/// The compact headroom readout: a percentage while the figure holds, the state
/// in words once it doesn't. The full story — window, model, reset time,
/// whether the figure was inferred — rides the tooltip.
function Headroom({ usage, now }: { usage: UsageSnapshot | null; now: number }) {
  const { display, label, title } = usageIndicator(usage, now);
  return (
    <span
      className={`agent-chip__usage agent-chip__usage--${display}`}
      title={title}
    >
      {label}
    </span>
  );
}
