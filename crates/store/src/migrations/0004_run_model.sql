-- Per-run model override (M7): the specific model an agent was asked to run
-- with (e.g. Claude's "opus", "sonnet", or a pinned version string). NULL
-- means the agent's own default, so existing runs are unaffected.
ALTER TABLE runs ADD COLUMN model TEXT;

-- Carry the same override through a scheduled rate-limit resume, so a resumed
-- pass keeps the model it was originally launched with.
ALTER TABLE pending_resumes ADD COLUMN model TEXT;
