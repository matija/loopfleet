// The defaults each settings section resets to, and the "is this section
// already at its defaults?" tests that keep the Reset control honest.
//
// `DEFAULT_SETTINGS` mirrors `store::Settings::default` (crates/store/src/
// settings.rs) — the same values the backend fills in for any key the user has
// never saved. Keeping a copy here is what lets Reset be instant and local:
// the panel restores defaults into the form and the user reviews them before
// pressing Save, so nothing round-trips to the backend to find out what a
// default is. The two must be changed together.
//
// The comparisons live here rather than in the panel because they're the
// fiddly part: a section counts as "at defaults" only when both what a save
// would persist and what the user can see match, which for retention means
// looking at the raw hour draft as well as the number it encodes to.

import type { Settings } from "./types";
import {
  DEFAULT_RETENTION_HOURS,
  retentionModeOf,
  retentionValue,
  type RetentionMode,
} from "./retention";
import { DEFAULT_THEME_ID, type ThemeId } from "./themes";

/// Mirrors `store::Settings::default`.
export const DEFAULT_SETTINGS: Settings = {
  default_agent: "claude",
  default_iterations: 1,
  concurrency_cap: 3,
  worktree_retention_hours: DEFAULT_RETENTION_HOURS,
};

/// The "Run defaults" section: the three fields the launch affordance reads.
export type RunDefaultsDraft = Pick<
  Settings,
  "default_agent" | "default_iterations" | "concurrency_cap"
>;

/// The "Worktrees" section as the panel edits it — a mode plus the raw hour
/// draft, not the single number they fold into (see retention.ts).
export type WorktreesDraft = {
  mode: RetentionMode;
  hours: string;
};

export const DEFAULT_RUN_DEFAULTS: RunDefaultsDraft = {
  default_agent: DEFAULT_SETTINGS.default_agent,
  default_iterations: DEFAULT_SETTINGS.default_iterations,
  concurrency_cap: DEFAULT_SETTINGS.concurrency_cap,
};

export const DEFAULT_WORKTREES: WorktreesDraft = {
  mode: retentionModeOf(DEFAULT_SETTINGS.worktree_retention_hours),
  hours: String(DEFAULT_RETENTION_HOURS),
};

export function isRunDefaultsAtDefault(draft: RunDefaultsDraft): boolean {
  return (
    draft.default_agent === DEFAULT_RUN_DEFAULTS.default_agent &&
    draft.default_iterations === DEFAULT_RUN_DEFAULTS.default_iterations &&
    draft.concurrency_cap === DEFAULT_RUN_DEFAULTS.concurrency_cap
  );
}

/// True only when the section would save the default retention *and* shows it.
///
/// The second half matters because `retentionValue` falls back to the default
/// hour count for an unusable draft: with "after" selected and the hours field
/// empty or half-typed, a save would write 48 even though the field doesn't
/// say 48. Treating that as "already at defaults" would grey out the one
/// control that puts the visible draft back in agreement with it.
export function isWorktreesAtDefault(draft: WorktreesDraft): boolean {
  if (retentionValue(draft.mode, draft.hours) !== DEFAULT_SETTINGS.worktree_retention_hours) {
    return false;
  }
  return draft.mode !== "after" || draft.hours.trim() === DEFAULT_WORKTREES.hours;
}

/// The "Appearance" section holds one field, and unlike the sections above it
/// isn't backend state — resetting it applies at once rather than waiting for
/// Save (see the section's own hint).
export function isThemeAtDefault(themeId: ThemeId): boolean {
  return themeId === DEFAULT_THEME_ID;
}
