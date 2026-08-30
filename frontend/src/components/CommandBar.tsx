// The live run view's `WHERE …`-style client-side event filter,
// freshness stamp, and agent status pill. Portals into the shell's toolbar
// (`.toolbar__filter`, see Toolbar) so the run view has exactly one control
// strip instead of the toolbar plus a redundant per-tab bar underneath it.
// Every slot is optional — a task tab has no event stream (no filter, no
// freshness) — so only the pieces the host supplies render. It is purely
// presentational; filter state, agent status, and freshness source all live
// in the host.
//
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";

export function CommandBar({
  toolbarFilter,
  filter,
  agent,
  since,
}: {
  /// The toolbar's filter-slot DOM node to portal into; renders nothing until set.
  toolbarFilter: HTMLElement | null;
  /// The `WHERE …` client-side event filter, when the tab has an event stream.
  filter?: { value: string; onChange: (v: string) => void };
  /// Agent connection pill: `connected` reflects `agent_status.installed`.
  agent?: {
    name: string;
    connected: boolean;
  };
  /// Epoch ms of the last activity to stamp "Xs ago" against; omit to hide.
  since?: number;
}) {
  if (!toolbarFilter) return null;
  return createPortal(
    <>
      {filter && (
        <input
          className="toolbar__filter-input"
          type="text"
          placeholder="WHERE event type or text…"
          aria-label="Filter events"
          value={filter.value}
          onChange={(e) => filter.onChange(e.target.value)}
        />
      )}
      <div className="toolbar__status">
        {since !== undefined && <Freshness since={since} />}
        {agent && (
          <span
            className={`toolbar__agent toolbar__agent--${
              agent.connected ? "on" : "off"
            }`}
            title={`Agent ${agent.name}: ${
              agent.connected ? "found on PATH" : "not installed"
            }`}
          >
            <span className="toolbar__agent-dot" />
            {agent.name} · {agent.connected ? "Connected" : "missing"}
          </span>
        )}
      </div>
    </>,
    toolbarFilter,
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
    <span className="toolbar__ago" aria-label="Time since last event">
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
