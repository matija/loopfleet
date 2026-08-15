-- Record when a finished run's on-disk footprint (worktree, sandbox profile,
-- progress dir) was reaped (unix millis). NULL until reaped; a run is only
-- eligible for reaping once it's terminal and has no pending resume.
ALTER TABLE runs ADD COLUMN reaped_at INTEGER;
