// Run subtabs (Workbench task 7): the database-client "Data / Privileges" subtab
// analog for a run tab — an Events / Diff / Files bar. Events hosts the typed
// event grid, Diff the per-iteration diff/patch viewer, Files the changed-files
// list. Shared by the live run view and the run timeline. Each host supplies
// only the tabs it has, so they never crowd each other: the live view shows
// Events + Files (its diffs land in the timeline), the timeline shows
// Events + Diff (the Diff panel already lists every changed file). An omitted
// key hides the tab entirely; a null count keeps the tab but hides its badge.

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
  counts: Partial<Record<RunSubtab, number | null>>;
}) {
  const shown = ITEMS.filter((it) => it.key in counts);
  return (
    <div className="run-subtabs" role="tablist" aria-label="Run views">
      {shown.map((it) => (
        <button
          key={it.key}
          role="tab"
          aria-selected={active === it.key}
          className={`run-subtab${active === it.key ? " run-subtab--active" : ""}`}
          onClick={() => onSelect(it.key)}
        >
          {it.label}
          {typeof counts[it.key] === "number" && (
            <span className="run-subtab__count">{counts[it.key]}</span>
          )}
        </button>
      ))}
    </div>
  );
}
