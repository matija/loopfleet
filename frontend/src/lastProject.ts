// The id of the last project the user had selected, persisted under a single
// localStorage key so the app can restore it on the next load. Pure
// read/write helpers — no React state — so callers can adopt them into
// whatever component owns project selection.

const STORAGE_KEY = "loopfleet.lastProject";

/// Reads the last-selected project id, answering `null` when the storage
/// entry is absent, malformed, or storage itself is unreadable (private
/// mode, disabled, etc.).
export function readLastProject(): string | null {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
  if (!raw) return null;
  return raw;
}

/// Persists `projectId` as the last-selected project.
export function writeLastProject(projectId: string): void {
  try {
    localStorage.setItem(STORAGE_KEY, projectId);
  } catch {
    // localStorage unavailable (private mode, quota) — preference just
    // doesn't persist across reloads.
  }
}

/// Clears the last-selected project, e.g. once it's confirmed gone from the
/// loaded project list so a stale id isn't retried on the next restore.
export function clearLastProject(): void {
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    // localStorage unavailable — nothing to clear.
  }
}
