// One shared per-agent headroom store for every surface that shows usage.
//
// `agentUsage()` probes the agent CLIs, so it is not something each component
// can call for itself: the agents panel, the run toolbar and the launch control
// would each spawn their own probes and could then disagree about the same
// agent. This module keeps the newest snapshot per agent as `agent_usage`
// events push updates and as explicit refreshes come in, and hands every
// subscriber the same map — so the figure beside the Run button is, by
// construction, the figure the agents panel is showing.

import type { UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import { agentUsage } from "./commands";
import { onAgentUsage } from "./events";
import type { UsageSnapshot } from "./types";

/// How often subscribers re-read the clock. Nothing pushes an event when a
/// snapshot merely *ages* past the staleness cutoff, so surfaces re-resolve on
/// their own; a minute is finer than the 15-minute cutoff it guards and coarse
/// enough to be invisible.
export const USAGE_TICK_MS = 60 * 1_000;

/// Latest snapshot per agent key (`AgentStatus.key`).
export type UsageByAgent = Record<string, UsageSnapshot>;

let snapshots: UsageByAgent = {};
const subscribers = new Set<(s: UsageByAgent) => void>();
let listener: Promise<UnlistenFn> | null = null;

function publish(next: UsageByAgent): void {
  snapshots = next;
  for (const notify of subscribers) notify(snapshots);
}

/// Begin listening (once). Kept alive for the app's lifetime rather than torn
/// down with the last subscriber: dropping the subscription would silently
/// miss the pushes that keep the cache honest.
function begin(): void {
  if (listener === null) {
    listener = onAgentUsage((snapshot) =>
      publish({ ...snapshots, [snapshot.agent_key]: snapshot }),
    );
  }
}

/// Re-probe every agent's headroom and publish what comes back. A deliberate
/// refresh — it spawns the agent CLIs — so call it on a surface whose whole
/// job is agent state, not on a timer. Failures are swallowed: a snapshot we
/// couldn't take leaves the surfaces reading "usage unknown" rather than
/// putting an error where a nicety belongs.
export async function refreshAgentUsage(): Promise<void> {
  try {
    const list = await agentUsage();
    // Pushed snapshots that landed while the probe was in flight are newer
    // than its answer, so they win the merge.
    publish({
      ...Object.fromEntries(list.map((s) => [s.agent_key, s])),
      ...snapshots,
    });
  } catch {
    // Leave the cache as it was.
  }
}

/// Subscribe to the shared store: the snapshots themselves plus the ticking
/// `now` the display helpers in `usage.ts` need to resolve staleness.
export function useAgentUsage(): { snapshots: UsageByAgent; now: number } {
  const [snaps, setSnaps] = useState<UsageByAgent>(snapshots);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    begin();
    subscribers.add(setSnaps);
    // Anything that landed before this mount is already in the cache.
    setSnaps(snapshots);
    return () => {
      subscribers.delete(setSnaps);
    };
  }, []);

  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), USAGE_TICK_MS);
    return () => clearInterval(id);
  }, []);

  return { snapshots: snaps, now };
}
