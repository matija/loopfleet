// ⌘K command palette — the global keyboard-first navigator (PRD task: "⌘K
// command palette. A global palette that fuzzy-searches projects, tasks, and
// runs and opens the match in the main pane, plus quick actions (add project,
// open settings). Esc closes.").
//
// Items come from three live sources plus a couple of quick actions:
//   - Projects: the registered repos (open a project's plan).
//   - Tasks: every task across every project, loaded lazily on open via
//     `plan_overview` (the backend exposes no global task list, and the PRD
//     forbids widening the command surface, so we fan out client-side).
//   - Runs: the session dock registry (open a run's live/timeline view).
//   - Actions: add a project (the folder picker), go to the overview/settings.
//
// Ranking is the subsequence matcher in `fuzzy.ts`, run against each item's
// title + subtitle. Arrow keys move the selection, Enter activates, Esc closes.
// All keyboard handling is scoped to the palette while it is open.

import { useEffect, useMemo, useRef, useState } from "react";
import { planOverview } from "../commands";
import { normalizeDisplayText } from "../displayText";
import { fuzzyMatch } from "../fuzzy";
import type { PlanView as Plan, Project, RunStatus } from "../types";
import type { ActiveRun } from "./RunDock";

/// What the palette needs to open a task in the main pane.
export type PaletteOpenTask = {
  projectId: string;
  planId: string;
  taskAnchor: string;
  taskText: string;
};

export type CommandPaletteProps = {
  open: boolean;
  onClose: () => void;
  projects: Project[];
  runs: ActiveRun[];
  onOpenProject: (projectId: string) => void;
  onOpenTask: (task: PaletteOpenTask) => void;
  onOpenRun: (runId: string) => void;
  onAddProject: () => void;
  onOpenOverview: () => void;
};

type Item = {
  id: string;
  group: string;
  title: string;
  subtitle?: string;
  hint?: string;
  run: () => void;
};

const STATUS_LABEL: Record<RunStatus, string> = {
  queued: "Queued",
  running: "Running",
  completed: "Completed",
  failed: "Failed",
  stopped: "Stopped",
};

function repoName(path: string): string {
  const parts = path.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || path;
}

export function CommandPalette({
  open,
  onClose,
  projects,
  runs,
  onOpenProject,
  onOpenTask,
  onOpenRun,
  onAddProject,
  onOpenOverview,
}: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const [tasks, setTasks] = useState<
    { projectId: string; planId: string; planLabel: string; anchor: string; text: string }[]
  >([]);
  // True while the per-project `plan_overview` fan-out is in flight, so the
  // Tasks group can signal it is still indexing rather than reading as empty.
  const [tasksLoading, setTasksLoading] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  // Fan out `plan_overview` across all projects when the palette opens, so the
  // task index is fresh each time. The project set is small (v1: a handful of
  // repos), so this is a few parallel calls. Failures per-project are swallowed
  // — a missing repo shouldn't blank the whole palette.
  useEffect(() => {
    if (!open) return;
    setQuery("");
    setSelected(0);
    setTasksLoading(true);
    let cancelled = false;
    Promise.all(
      projects.map((p) =>
        planOverview(p.id)
          .then((plans: Plan[]) =>
            plans.flatMap((plan) =>
              plan.tasks.map((t) => ({
                projectId: p.id,
                planId: plan.plan_id,
                planLabel: plan.title ?? plan.file_path,
                anchor: t.anchor,
                text: t.text,
              })),
            ),
          )
          .catch(() => [] as typeof tasks),
      ),
    ).then((groups) => {
      if (cancelled) return;
      setTasks(groups.flat());
      setTasksLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, [open, projects]);

  // Focus the input on open.
  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  const items = useMemo<Item[]>(() => {
    const actions: Item[] = [
      {
        id: "act:add",
        group: "Actions",
        title: "Add project…",
        subtitle: "Pick a git repo to register",
        hint: "folder picker",
        run: onAddProject,
      },
      {
        id: "act:overview",
        group: "Actions",
        title: "Go to overview",
        subtitle: "Agents, settings, sandbox boundary",
        hint: "home",
        run: onOpenOverview,
      },
    ];
    const projectItems: Item[] = projects.map((p) => ({
      id: `proj:${p.id}`,
      group: "Projects",
      title: repoName(p.repo_path),
      subtitle: p.repo_path,
      hint: "project",
      run: () => onOpenProject(p.id),
    }));
    const taskItems: Item[] = tasks.map((t) => ({
      id: `task:${t.projectId}:${t.planId}:${t.anchor}`,
      group: "Tasks",
      title: normalizeDisplayText(t.text),
      subtitle: t.planLabel,
      hint: "task",
      run: () =>
        onOpenTask({
          projectId: t.projectId,
          planId: t.planId,
          taskAnchor: t.anchor,
          taskText: t.text,
        }),
    }));
    const runItems: Item[] = runs.map((r) => ({
      id: `run:${r.runId}`,
      group: "Runs",
      title: normalizeDisplayText(r.taskText),
      subtitle: `${STATUS_LABEL[r.status]} · ${r.agent} · ${r.projectName}`,
      hint: r.status,
      run: () => onOpenRun(r.runId),
    }));
    return [...actions, ...projectItems, ...taskItems, ...runItems];
  }, [projects, tasks, runs, onAddProject, onOpenOverview, onOpenProject, onOpenTask, onOpenRun]);

  // Filter + rank by the best match against the title or subtitle.
  const results = useMemo(() => {
    const q = query.trim();
    if (!q) return items;
    const scored: { item: Item; score: number; indices: number[] }[] = [];
    for (const item of items) {
      const titleM = fuzzyMatch(q, item.title);
      const subM = item.subtitle ? fuzzyMatch(q, item.subtitle) : null;
      const best =
        titleM.matched && (subM === null || !subM.matched || titleM.score >= subM.score)
          ? { score: titleM.score, indices: titleM.indices }
          : subM && subM.matched
            ? { score: subM.score, indices: subM.indices }
            : null;
      if (best) scored.push({ item, ...best });
    }
    scored.sort((a, b) => b.score - a.score);
    return scored.map((s) => s.item);
  }, [items, query]);

  // Clamp selection when the result set shrinks.
  useEffect(() => {
    setSelected((s) => Math.min(s, Math.max(0, results.length - 1)));
  }, [results.length]);

  // Scroll the selected row into view.
  useEffect(() => {
    if (!open) return;
    const el = listRef.current?.querySelector<HTMLElement>(
      `[data-idx="${selected}"]`,
    );
    el?.scrollIntoView({ block: "nearest" });
  }, [selected, open]);

  if (!open) return null;

  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelected((s) => Math.min(s + 1, results.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelected((s) => Math.max(s - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const item = results[selected];
      if (item) {
        item.run();
        onClose();
      }
    }
  }

  // Group the ranked results for display, preserving the ranked order within
  // each group and a stable group order.
  const groupOrder = ["Actions", "Projects", "Tasks", "Runs"];
  const grouped = groupOrder
    .map((g) => ({ group: g, rows: results.filter((r) => r.group === g) }))
    .filter((g) => g.rows.length > 0);
  let runningIdx = 0;
  const ranked = grouped.flatMap((g) =>
    g.rows.map((item) => ({ item, idx: runningIdx++ })),
  );

  return (
    <div className="palette__overlay" role="dialog" aria-modal="true" aria-label="Command palette">
      <div className="palette">
        <input
          ref={inputRef}
          className="palette__input"
          type="text"
          placeholder="Search projects, tasks, runs, actions…"
          aria-label="Command palette query"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKeyDown}
        />
        <div className="palette__list" ref={listRef}>
          {ranked.length === 0 ? (
            <div className="palette__empty">
              No matches for “{query.trim()}”.
            </div>
          ) : (
            grouped.map((g) => (
              <div key={g.group} className="palette__group">
                <div className="palette__group-label">{g.group}</div>
                {g.rows.map((item) => {
                  const idx = results.indexOf(item);
                  const active = idx === selected;
                  return (
                    <button
                      key={item.id}
                      data-idx={idx}
                      className={`palette__row${active ? " palette__row--active" : ""}`}
                      aria-current={active}
                      onMouseEnter={() => setSelected(idx)}
                      onClick={() => {
                        item.run();
                        onClose();
                      }}
                    >
                      <span className="palette__row-body">
                        <span className="palette__row-title">{item.title}</span>
                        {item.subtitle && (
                          <span className="palette__row-sub">{item.subtitle}</span>
                        )}
                      </span>
                      {item.hint && (
                        <span className="palette__row-hint">{item.hint}</span>
                      )}
                    </button>
                  );
                })}
              </div>
            ))
          )}
        </div>
        <div className="palette__foot">
          {tasksLoading && (
            <span className="palette__foot-loading">Indexing tasks…</span>
          )}
          <span><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
          <span><kbd>↵</kbd> open</span>
          <span><kbd>esc</kbd> close</span>
        </div>
      </div>
    </div>
  );
}
