-- Initial schema for the loopfleet data model (PRD "Data model" section).
-- Per-task live state (TaskStatus) is DERIVED from run records at read time and
-- is intentionally NOT stored here. `checked` is the authored "implemented"
-- baseline (read as `Accepted` by `derive_status`), not a live signal.

CREATE TABLE projects (
    id              TEXT PRIMARY KEY,
    repo_path       TEXT NOT NULL UNIQUE,
    plan_convention TEXT NOT NULL           -- 'prd' | 'folder'
);

CREATE TABLE plans (
    id         TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    file_path  TEXT NOT NULL
);

-- A task is keyed by its anchor's normalized text within a plan; line_hint is a
-- tiebreaker, never the key (PRD: Plans).
CREATE TABLE tasks (
    plan_id         TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    normalized_text TEXT NOT NULL,
    line_hint       INTEGER,
    text            TEXT NOT NULL,
    checked         INTEGER NOT NULL,       -- authored "implemented" baseline, 0/1
    PRIMARY KEY (plan_id, normalized_text)
);

-- Model B: one run, one task. task_ref = (plan_id, task_anchor).
CREATE TABLE runs (
    id             TEXT PRIMARY KEY,
    plan_id        TEXT NOT NULL,
    task_anchor    TEXT NOT NULL,           -- task's normalized_text
    agent          TEXT NOT NULL,
    worktree_path  TEXT,
    branch         TEXT,
    sb_profile     TEXT,
    progress_path  TEXT,                    -- external, app-managed, keyed by run-id
    max_iterations INTEGER NOT NULL,
    status         TEXT NOT NULL,           -- queued|running|completed|failed|stopped
    accepted       INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (plan_id, task_anchor) REFERENCES tasks(plan_id, normalized_text)
);

CREATE TABLE iterations (
    run_id           TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    n                INTEGER NOT NULL,
    shadow_ref       TEXT,                  -- refs/agentapp/run-<id>/iter-<n>
    event_log_offset INTEGER,
    usage            TEXT,                  -- JSON
    exit             INTEGER,               -- agent process exit code only
    PRIMARY KEY (run_id, n)
);

-- M5, deferred. Schema defined now to match the data model.
CREATE TABLE sessions (
    id         TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    agent      TEXT NOT NULL,
    plan_file  TEXT NOT NULL,
    status     TEXT NOT NULL
);

-- Append-only normalized event log for both runs and sessions.
CREATE TABLE events (
    seq                   INTEGER PRIMARY KEY AUTOINCREMENT,
    run_or_session_id     TEXT NOT NULL,
    normalized_event_json TEXT NOT NULL,
    ts                    INTEGER NOT NULL   -- unix millis
);

CREATE INDEX idx_events_owner ON events(run_or_session_id, seq);
CREATE INDEX idx_runs_task    ON runs(plan_id, task_anchor);
CREATE INDEX idx_plans_project ON plans(project_id);
