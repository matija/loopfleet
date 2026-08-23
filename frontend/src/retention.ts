// Worktree retention encoding shared by the settings surface. The backend
// stores a single number (`store::Settings::worktree_retention_hours`) that
// carries three distinct modes; the UI edits them as a mode plus an hour
// count, so the mapping between the two lives here, pure and testable.

/// How long a finished run's worktree survives before the sweep reaps it:
/// `immediately` (stored `0`), `after` N hours (stored N), or `never`
/// (stored `-1`). Accepted runs are swept regardless of the mode.
export type RetentionMode = "immediately" | "after" | "never";

/// The hour count applied when the mode is `after` but the user's draft
/// doesn't name a usable one. Matches `store::Settings::default`.
export const DEFAULT_RETENTION_HOURS = 48;

/// Which mode a stored `worktree_retention_hours` represents. Anything
/// positive is `after`; anything negative is treated as `never`, so a value
/// written by a newer build (or hand-edited in the DB) still lands on a mode
/// the picker can show rather than falling through to `after`.
export function retentionModeOf(hours: number): RetentionMode {
  if (hours < 0) return "never";
  if (hours === 0) return "immediately";
  return "after";
}

/// Fold an edited mode and its raw hour-count draft back into the stored
/// number. The draft is a string because it comes straight from the input:
/// an empty or non-numeric one falls back to `DEFAULT_RETENTION_HOURS`
/// rather than becoming `0`, which would silently mean "reap immediately" —
/// the opposite of what someone mid-edit intends. Fractional hours are
/// floored, since the sweep measures in whole hours.
export function retentionValue(mode: RetentionMode, hoursDraft: string): number {
  if (mode === "never") return -1;
  if (mode === "immediately") return 0;
  const parsed = Number(hoursDraft.trim());
  return Number.isFinite(parsed) && parsed >= 1
    ? Math.floor(parsed)
    : DEFAULT_RETENTION_HOURS;
}
