// Per-project launch preferences (agent + pass count), persisted under one
// localStorage key per project so the launch control remembers the last
// choice made for that project. Pure read/write helpers — no React state —
// so callers can adopt them into whatever component owns the launch UI.

const DEFAULT_AGENT = "claude";
const DEFAULT_PASSES = 1;

export type LaunchPrefs = {
  agent: string;
  /// Model override to launch with, or "" for the agent's own default.
  model: string;
  passes: number;
};

function storageKey(projectId: string): string {
  return `loopfleet.launch.${projectId}`;
}

/// Reads the effective launch prefs for `projectId`, falling back to
/// `{ agent: "claude", passes: 1 }` piece-by-piece when the stored value is
/// absent, malformed, or names an agent that isn't in `installedAgents`
/// (e.g. it was uninstalled since the preference was saved).
export function readLaunchPrefs(
  projectId: string,
  installedAgents: string[],
): LaunchPrefs {
  const fallback = { agent: DEFAULT_AGENT, model: "", passes: DEFAULT_PASSES };
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(storageKey(projectId));
  } catch {
    return fallback;
  }
  if (!raw) return fallback;

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return fallback;
  }
  if (typeof parsed !== "object" || parsed === null) {
    return fallback;
  }

  const stored = parsed as Record<string, unknown>;
  const agent =
    typeof stored.agent === "string" && installedAgents.includes(stored.agent)
      ? stored.agent
      : DEFAULT_AGENT;
  const model = typeof stored.model === "string" ? stored.model : "";
  const passes =
    typeof stored.passes === "number" &&
    Number.isFinite(stored.passes) &&
    stored.passes >= 1
      ? Math.floor(stored.passes)
      : DEFAULT_PASSES;

  return { agent, model, passes };
}

/// Persists `prefs` as the launch preference for `projectId`.
export function writeLaunchPrefs(projectId: string, prefs: LaunchPrefs): void {
  try {
    localStorage.setItem(storageKey(projectId), JSON.stringify(prefs));
  } catch {
    // localStorage unavailable (private mode, quota) — preference just
    // doesn't persist across reloads.
  }
}

/// The agent a bare `Run` would start for `projectId` — the stored preference
/// resolved against `installedAgents`, exactly as [`readLaunchPrefs`] resolves
/// it.
///
/// The launch control needs this answer before the user has touched the agent
/// picker, so its headroom readout describes the agent the button would
/// actually launch rather than nothing at all while the preference is being
/// adopted into state.
export function preferredAgent(
  projectId: string,
  installedAgents: string[],
): string {
  return readLaunchPrefs(projectId, installedAgents).agent;
}
