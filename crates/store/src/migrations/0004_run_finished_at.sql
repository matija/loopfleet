-- When a run reaches a terminal status (completed/failed/stopped/limit-reached),
-- record the wall-clock time it finished (unix millis). NULL while the run is
-- still queued or running.
ALTER TABLE runs ADD COLUMN finished_at INTEGER;
