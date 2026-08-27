// Agent CLI availability + version-drift chips. One chip per v1 agent: green +
// version when installed, a yellow "tested <v>" drift warning when the detected
// version differs from the one the adapter was tested against, red + reason when
// missing (a run with a missing CLI is refused, so this is a launch guardrail).
//
// Each installed agent's chip also carries its limit headroom beside the
// version, so the same glance that answers "can I launch this agent?" answers
// "will it get anywhere?". The snapshots come from the shared store
// (`agentUsage.ts`) that the run toolbar and the launch control read too, so
// every surface quotes the same figure; the wording and bucketing are
// `usage.ts`'s. This file only refreshes and renders.
//
// The panel opens in the shared vocabulary — `panel__head` led by the same
// glyph the overview card carries, then one `panel__lead` line saying what the
// chips are for — so Agents, Settings and the sandbox panel all start the same
// way rather than each opening in its own dialect.

import { useEffect, useState } from "react";
import { refreshAgentUsage, useAgentUsage } from "../agentUsage";
import { agentStatus } from "../commands";
import type { AgentStatus, UsageSnapshot } from "../types";
import { formatCountdown, formatResetTime, usageIndicator } from "../usage";
import { AgentIcon } from "./Icon";

export function AgentStatusPanel() {
  // `loaded` distinguishes "fetching" from "fetched an empty set" — without it,
  // an in-flight load renders an empty chip row that reads as "no agents".
  const [agents, setAgents] = useState<AgentStatus[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { snapshots, now } = useAgentUsage();

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
  // This panel is the one surface whose whole job is agent state, so it is
  // where a fresh probe is worth spawning; every other reader lives off the
  // pushes that follow.
  useEffect(() => {
    void refreshAgentUsage();
  }, []);

  return (
    <section className="panel">
      <div className="panel__head">
        <AgentIcon size={16} className="icon panel__icon" />
        <h3>Agents</h3>
      </div>
      <p className="panel__lead">
        The agent CLIs installed on this machine. A run whose CLI is missing is
        refused at launch, so this is the first thing to check when one won’t
        start.
      </p>
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
              usage={snapshots[a.key] ?? null}
              now={now}
            />
          ))}
        </div>
      )}
    </section>
  );
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
/// in words once it doesn't, plus — once the CLI's prose resolves one — a
/// reset countdown beside it. The full story — window, model, exact reset
/// instant, whether the figure was inferred — still rides the tooltip.
function Headroom({ usage, now }: { usage: UsageSnapshot | null; now: number }) {
  const { display, label, title } = usageIndicator(usage, now);
  const resetAtMs = display !== "unknown" ? (usage?.reset_at_ms ?? null) : null;
  return (
    <>
      <span
        className={`agent-chip__usage agent-chip__usage--${display}`}
        title={title}
      >
        {label}
      </span>
      {resetAtMs !== null && (
        <span
          className="agent-chip__reset"
          title={`Resets ${formatResetTime(resetAtMs, now)}`}
        >
          resets {formatCountdown(resetAtMs, now)}
        </span>
      )}
    </>
  );
}
