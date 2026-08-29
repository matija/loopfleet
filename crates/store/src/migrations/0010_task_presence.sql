-- Per-task presence, so "the plan file was replaced" can be told from "one task
-- was reworded" — the two cases the positional binding signal cannot otherwise
-- distinguish, because a deleted task and a rewritten one leave the same trace
-- (a stored anchor matching nothing, with a line that still lands in the file).
--
-- `present` is whether the task was in the file at the last sync. `positional`
-- is whether its last known line may still be offered as evidence for binding a
-- run. A task that vanishes in an edit that most tasks survived was reworded and
-- keeps `positional`; a task that vanishes together with every other task in the
-- plan belonged to a plan that is gone, and loses it permanently.
--
-- Both default to 0 for rows that already exist: there is no presence record for
-- anything synced before this migration, so the whole of that history is
-- declared positionally unrecoverable rather than guessed at. Text binding
-- (exact and legacy) is unaffected, and it is what carries the overwhelming
-- majority of bindings; only the weakest signal is withdrawn, and only for rows
-- whose provenance is unknown. Rows synced from here on are written with both
-- set to 1.
ALTER TABLE tasks ADD COLUMN present INTEGER NOT NULL DEFAULT 0;
ALTER TABLE tasks ADD COLUMN positional INTEGER NOT NULL DEFAULT 0;
