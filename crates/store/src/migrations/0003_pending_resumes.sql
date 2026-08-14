-- Pending resumes: a scheduled re-launch for a run that stopped short of its
-- task, so the app can pick it back up (e.g. after a crash) without losing the
-- schedule. One row per outstanding resume; deleted when it fires or is
-- cancelled (see store::pending_resumes).
CREATE TABLE pending_resumes (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id         TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    task_anchor    TEXT NOT NULL,
    agent          TEXT NOT NULL,
    pass_count     INTEGER NOT NULL,
    resume_at      INTEGER NOT NULL,        -- unix millis
    attempt        INTEGER NOT NULL
);

CREATE INDEX idx_pending_resumes_run ON pending_resumes(run_id);
