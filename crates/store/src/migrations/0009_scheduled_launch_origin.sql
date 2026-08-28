-- Record why a launch was scheduled: a user-initiated "start this task at
-- 06:00" (`manual`) versus autopilot chaining the plan's next task after an
-- auto-merge (`auto_advance`). Lets the UI and history distinguish the two.
ALTER TABLE scheduled_launches ADD COLUMN origin TEXT NOT NULL DEFAULT 'manual';
