-- Settings (PRD M6): global app defaults + per-project sandbox overrides.

-- Global key/value settings. One row per key; missing keys fall back to the
-- code-side defaults in `settings.rs`, so this table only holds overrides the
-- user has explicitly saved.
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Per-project sandbox write-boundary overrides: newline-separated absolute
-- paths added to the run's Seatbelt write grants. Empty = none. The parent
-- repo's `.git` must never be listed here (commits are app-owned); paths are
-- validated absolute at launch, and the rendered profile is shown in the UI.
ALTER TABLE projects ADD COLUMN sandbox_extra_writes TEXT NOT NULL DEFAULT '';
