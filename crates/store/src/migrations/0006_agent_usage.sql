-- Per-agent limit headroom: the most recent rate-limit observation the app saw
-- for an agent, so a launch can tell whether that agent is currently exhausted
-- without replaying the event log. One row per agent (the key), overwritten on
-- every `RateLimited` event — this is a latest-observation snapshot, not a
-- history (see store::agent_usage).
CREATE TABLE agent_usage (
    agent       TEXT PRIMARY KEY,
    reset_at    TEXT,                   -- ISO-8601 the agent reported, NULL if it gave none
    message     TEXT,                   -- agent-supplied detail, NULL if it gave none
    observed_at INTEGER NOT NULL        -- when the app saw the event, unix millis
);
