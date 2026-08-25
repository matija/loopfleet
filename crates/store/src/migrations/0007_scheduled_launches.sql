-- Scheduled launches: a run the user asked for later ("start this task at
-- 06:00"), recorded before any run exists. Unlike pending_resumes — which
-- re-launches an existing run — a scheduled launch is keyed to the plan task
-- it will launch, so it survives having no run yet. The row is deleted when
-- the launch fires or is cancelled (see store::scheduled_launches).
CREATE TABLE scheduled_launches (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    plan_id     TEXT NOT NULL,
    task_anchor TEXT NOT NULL,            -- task's normalized_text
    agent       TEXT NOT NULL,
    model       TEXT,                     -- NULL = the agent's own default
    pass_count  INTEGER NOT NULL,
    launch_at   INTEGER NOT NULL,         -- unix millis
    created_at  INTEGER NOT NULL DEFAULT (unixepoch() * 1000),
    FOREIGN KEY (plan_id, task_anchor) REFERENCES tasks(plan_id, normalized_text) ON DELETE CASCADE
);

CREATE INDEX idx_scheduled_launches_task ON scheduled_launches(plan_id, task_anchor);
CREATE INDEX idx_scheduled_launches_due ON scheduled_launches(launch_at);
