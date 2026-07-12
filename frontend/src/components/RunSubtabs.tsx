// Run subtabs (Workbench task 7): the database-client "Data / Privileges" subtab
// analog for a run tab — an Events / Diff / Files bar. Events hosts the typed
// event grid, Diff the per-iteration diff/patch viewer, Files the changed-files
// list. Shared by the live run view and the run timeline so both organize their
// bodies the same way. A null count hides the badge (e.g. a live run has no
// per-iteration diffs to count).

export type RunSubtab = "events" | "diff" | "files";

const ITEMS: { key: RunSubtab; label: string }[] = [
  { key: "events", label: "Events" },
  { key: "diff", label: "Diff" },
  { key: "files", label: "Files" },
];

export function RunSubtabs({
  active,
  onSelect,
  counts,
}: {
  active: RunSubtab;
  onSelect: (t: RunSubtab) => void;
  counts: Record<RunSubtab, number | null>;
}) {
  return (
    <div className="run-subtabs" role="tablist" aria-label="Run views">
      {ITEMS.map((it) => (
        <button
          key={it.key}
          role="tab"
          aria-selected={active === it.key}
          className={`run-subtab${active === it.key ? " run-subtab--active" : ""}`}
          onClick={() => onSelect(it.key)}
        >
          {it.label}
          {counts[it.key] !== null && (
            <span className="run-subtab__count">{counts[it.key]}</span>
          )}
        </button>
      ))}
    </div>
  );
}
