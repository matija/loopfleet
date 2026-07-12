// Per-tab command bar (the DB client's per-tab command row). A slim strip at the
// top of a run/task tab carrying, left to right: an object-name pill (the task),
// a `WHERE …`-style client-side event filter, then a right cluster with a live
// "Xs ago" freshness stamp, an agent "Connected"/"missing" status pill, and the
// Run / Re-run action slot. Every slot is optional — a task tab has no event
// stream (no filter, no freshness), a run tab has no re-run task context — so the
// bar renders only the pieces its host supplies. It is purely presentational;
// filter state, launch logic, and freshness source all live in the host.

import { useEffect, useState, type ReactNode } from "react";
import { normalizeDisplayText } from "../displayText";

export function CommandBar({
  task,
  filter,
  agent,
  since,
  children,
}: {
  /// Object-name pill text — the task the tab is bound to.
  task: string;
  /// The `WHERE …` client-side event filter, when the tab has an event stream.
  filter?: { value: string; onChange: (v: string) => void };
  /// Agent connection pill: `connected` reflects `agent_status.installed`.
  agent?: { name: string; connected: boolean };
  /// Epoch ms of the last activity to stamp "Xs ago" against; omit to hide.
  since?: number;
  /// The Run / Re-run control, rendered at the far right.
  children?: ReactNode;
}) {
  return (
    <div className="command-bar">
      <span className="command-bar__task" title={normalizeDisplayText(task)}>
        {normalizeDisplayText(task)}
      </span>
      {filter && (
        <input
          className="command-bar__filter"
          type="text"
          placeholder="WHERE event type or text…"
          aria-label="Filter events"
          value={filter.value}
          onChange={(e) => filter.onChange(e.target.value)}
        />
      )}
      <div className="command-bar__right">
        {since !== undefined && <Freshness since={since} />}
        {agent && (
          <span
            className={`command-bar__agent command-bar__agent--${
              agent.connected ? "on" : "off"
            }`}
            title={`Agent ${agent.name}: ${
              agent.connected ? "found on PATH" : "not installed"
            }`}
          >
            <span className="command-bar__agent-dot" />
            {agent.name} · {agent.connected ? "Connected" : "missing"}
          </span>
        )}
        {children}
      </div>
    </div>
  );
}

// A live "Xs ago" stamp, ticking once a second against `since`. Mirrors the
// reference's freshness indicator: how stale the surface is relative to the last
// event that landed.
function Freshness({ since }: { since: number }) {
  const [, tick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(id);
  }, []);
  return (
    <span className="command-bar__ago" aria-label="Time since last event">
      {ago(since)}
    </span>
  );
}

function ago(since: number): string {
  const s = Math.max(0, Math.round((Date.now() - since) / 1000));
  if (s < 1) return "just now";
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  return `${m}m ago`;
}
