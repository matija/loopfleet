// Per-project launch preferences (agent + pass count), persisted under one
// localStorage key per project so the launch control remembers the last
// choice made for that project. Pure read/write helpers — no React state —
// so callers can adopt them into whatever component owns the launch UI.

const DEFAULT_AGENT = "claude";
const DEFAULT_PASSES = 1;

export type LaunchPrefs = {
  agent: string;
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
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(storageKey(projectId));
  } catch {
    return { agent: DEFAULT_AGENT, passes: DEFAULT_PASSES };
  }
  if (!raw) return { agent: DEFAULT_AGENT, passes: DEFAULT_PASSES };

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return { agent: DEFAULT_AGENT, passes: DEFAULT_PASSES };
  }
  if (typeof parsed !== "object" || parsed === null) {
    return { agent: DEFAULT_AGENT, passes: DEFAULT_PASSES };
  }

  const stored = parsed as Record<string, unknown>;
  const agent =
    typeof stored.agent === "string" && installedAgents.includes(stored.agent)
      ? stored.agent
      : DEFAULT_AGENT;
  const passes =
    typeof stored.passes === "number" &&
    Number.isFinite(stored.passes) &&
    stored.passes >= 1
      ? Math.floor(stored.passes)
      : DEFAULT_PASSES;

  return { agent, passes };
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
