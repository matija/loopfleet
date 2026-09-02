// Session-lifetime cache for the two backend answers that every plan/task
// surface asks for on mount, and that neither changes on its own.
//
// `agent_status` spawns `<binary> --version` for each known agent — measured at
// ~0.4s for the three of them on a warm machine — and `get_settings` is a round
// trip per caller. Both were fetched per component instance, so returning to
// the plan re-probed the agent CLIs and every task row's launch control asked
// for the settings separately. Navigating back to a 13-task plan meant one CLI
// probe sweep plus 13 identical settings calls, all of it repeated on the next
// visit. This module fetches each once and hands every caller the same promise.
//
// Same idea as the shared snapshot store in `agentUsage.ts`, one layer down:
// that one keeps surfaces from disagreeing, this one keeps them from repeating
// work. The cached promise is kept for the app's lifetime; the two things that
// can invalidate it are explicit — a settings save (`saveAppSettings`) and the
// agents panel's deliberate re-probe (`refreshAgentCatalog`).

import { agentStatus, getSettings, saveSettings } from "./commands";
import type { AgentStatus, Settings } from "./types";

let agents: Promise<AgentStatus[]> | null = null;
let settings: Promise<Settings> | null = null;

/// Every known agent and whether its CLI is installed. Probed once per session.
export function agentCatalog(): Promise<AgentStatus[]> {
  if (agents === null) {
    // A failed probe must not be cached as the session's answer — drop it so
    // the next caller retries rather than inheriting a one-off failure.
    agents = agentStatus().catch((e) => {
      agents = null;
      throw e;
    });
  }
  return agents;
}

/// Re-run agent discovery and adopt the result as the session's answer. For the
/// agents panel, whose whole job is agent state; every other reader lives off
/// the cache.
export function refreshAgentCatalog(): Promise<AgentStatus[]> {
  agents = null;
  return agentCatalog();
}

/// The global app settings. Read once per session; kept current by
/// `saveAppSettings`, the only writer.
export function appSettings(): Promise<Settings> {
  if (settings === null) {
    settings = getSettings().catch((e) => {
      settings = null;
      throw e;
    });
  }
  return settings;
}

/// Persist settings and adopt them as the cached answer, so surfaces that read
/// `appSettings` after a save see what was actually saved without a refetch.
export async function saveAppSettings(next: Settings): Promise<void> {
  await saveSettings(next);
  settings = Promise.resolve(next);
}
