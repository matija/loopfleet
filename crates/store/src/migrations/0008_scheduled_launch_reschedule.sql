-- Track how many times a scheduled launch has been pushed back because the
-- agent was still exhausted when it fired. Bounded in application code
-- (`MAX_LAUNCH_RESCHEDULES`); this column just carries the count across a
-- crash so the bound survives a restart mid-chain.
ALTER TABLE scheduled_launches ADD COLUMN reschedule_count INTEGER NOT NULL DEFAULT 0;
