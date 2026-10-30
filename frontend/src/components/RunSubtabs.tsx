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
  idPrefix,
}: {
  active: RunSubtab;
  onSelect: (t: RunSubtab) => void;
  counts: Partial<Record<RunSubtab, number | null>>;
  idPrefix: string;
}) {
  const shown = ITEMS.filter((it) => it.key in counts);
  return (
    <div className="run-subtabs" role="tablist" aria-label="Run views">
      {shown.map((it) => (
        <button
          key={it.key}
          role="tab"
          id={`${idPrefix}-${it.key}-tab`}
          aria-controls={`${idPrefix}-${it.key}-panel`}
          aria-selected={active === it.key}
          tabIndex={active === it.key ? 0 : -1}
          className={`run-subtab${active === it.key ? " run-subtab--active" : ""}`}
          onClick={() => onSelect(it.key)}
          onKeyDown={(event) => {
            if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || event.nativeEvent.isComposing) return;
            const index = shown.findIndex((item) => item.key === it.key);
            const next = event.key === "Home" ? 0 : event.key === "End" ? shown.length - 1 : event.key === "ArrowRight" ? (index + 1) % shown.length : event.key === "ArrowLeft" ? (index + shown.length - 1) % shown.length : null;
            if (next === null) return;
            event.preventDefault();
            onSelect(shown[next].key);
            document.getElementById(`${idPrefix}-${shown[next].key}-tab`)?.focus();
          }}
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
