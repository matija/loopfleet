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
import { taskSummary } from "../displayText";
import { fuzzyMatch } from "../fuzzy";
import { SHORTCUTS, shortcutKeyGlyphs } from "../shortcuts";
import type { PlanView as Plan, Project } from "../types";
import { isActiveRun, RUN_STATUS_LABEL } from "../status";
import type { ActiveRun } from "./RunDock";
import { ChecklistIcon, ComposeIcon, FolderIcon, PlayIcon, SearchIcon, TrashIcon } from "./Icon";

/// One glyph per result group, so a row's type reads at a glance before its
/// title does.
const GROUP_ICON: Record<string, typeof FolderIcon> = {
  Actions: ComposeIcon,
  Projects: FolderIcon,
  Tasks: ChecklistIcon,
  Runs: PlayIcon,
};

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
  /// The project currently selected in the sidebar, if any — what "Archive
  /// plan" targets. `null` when nothing is selected (the overview pane).
  currentProjectId: string | null;
  onOpenProject: (projectId: string) => void;
  onOpenTask: (task: PaletteOpenTask) => void;
  onOpenRun: (runId: string) => void;
  onAddProject: () => void;
  onOpenOverview: () => void;
  /// Opens the sidebar's remove-project confirmation for the given project,
  /// mirroring the trash icon on its sidebar row.
  onRemoveProject: (projectId: string) => void;
  /// Jumps to the current project's plan in the PRD view and starts the
  /// archive flow for it, mirroring PrdView's own Archive button.
  onArchivePlan: (projectId: string, planId: string) => void;
};

type Item = {
  id: string;
  group: string;
  title: string;
  subtitle?: string;
  hint?: string;
  run: () => void;
  /// Overrides the group's default icon (e.g. a per-project "Remove
  /// project" row still living in the Projects group, but drawn with a
  /// trash glyph so it doesn't read as another way to open the project).
  icon?: typeof FolderIcon;
  /// Set with the reason it's inert (e.g. "No plan to archive") — the row
  /// still renders and is readable, but Enter/click no-op and the reason
  /// shows in place of the hint.
  disabledReason?: string;
};

/// Footer hint rows for the palette's own (non-global) keys — arrow
/// navigation, Enter to open, Esc to close — rendered the same data-driven
/// way as the global shortcuts below them.
const NAV_HINTS: { keys: string[]; label: string }[] = [
  { keys: ["↑", "↓"], label: "navigate" },
  { keys: ["↵"], label: "open" },
  { keys: ["esc"], label: "close" },
];

function repoName(path: string): string {
  const parts = path.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || path;
}

export function CommandPalette({
  open,
  onClose,
  projects,
  runs,
  currentProjectId,
  onOpenProject,
  onOpenTask,
  onOpenRun,
  onAddProject,
  onOpenOverview,
  onRemoveProject,
  onArchivePlan,
}: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const [tasks, setTasks] = useState<
    { projectId: string; planId: string; planLabel: string; anchor: string; text: string }[]
  >([]);
  // The plans backing each project's tasks, keyed by project id — the same
  // `plan_overview` fan-out below, kept whole (not just flattened into
  // `tasks`) so "Archive plan" can name the current project's plan.
  const [plansByProject, setPlansByProject] = useState<Record<string, Plan[]>>({});
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
          .then((plans: Plan[]) => ({
            projectId: p.id,
            plans,
            tasks: plans.flatMap((plan) =>
              plan.tasks.map((t) => ({
                projectId: p.id,
                planId: plan.plan_id,
                planLabel: plan.title ?? plan.file_path,
                anchor: t.anchor,
                text: t.text,
              })),
            ),
          }))
          .catch(() => ({ projectId: p.id, plans: [] as Plan[], tasks: [] as typeof tasks })),
      ),
    ).then((results) => {
      if (cancelled) return;
      setTasks(results.flatMap((r) => r.tasks));
      setPlansByProject(Object.fromEntries(results.map((r) => [r.projectId, r.plans])));
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

  // "Archive plan" targets the current project's plan (its first synced plan
  // document — the common case is exactly one). Disabled with a stated
  // reason when there's no project selected, the project has no plan, or the
  // plan has a run still queued or running (the same guard `archive_plan`
  // itself enforces server-side, checked here against the same task/run data
  // the palette already indexes so the reason shows without a round trip).
  const archiveTarget = useMemo(() => {
    if (!currentProjectId) {
      return { planId: null as string | null, disabledReason: "No project selected" };
    }
    const plans = plansByProject[currentProjectId];
    if (!plans || plans.length === 0) {
      return { planId: null as string | null, disabledReason: "No plan to archive" };
    }
    const plan = plans[0];
    const activeAnchors = new Set(
      runs
        .filter((r) => isActiveRun(r.status) && r.projectId === currentProjectId)
        .map((r) => r.taskAnchor),
    );
    const hasActiveRun = tasks.some(
      (t) =>
        t.projectId === currentProjectId &&
        t.planId === plan.plan_id &&
        activeAnchors.has(t.anchor),
    );
    return {
      planId: plan.plan_id,
      disabledReason: hasActiveRun ? "Plan has a run still active" : undefined,
    };
  }, [currentProjectId, plansByProject, tasks, runs]);

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
      {
        id: "act:archive-plan",
        group: "Actions",
        title: "Archive plan",
        subtitle: archiveTarget.disabledReason ?? "Move the current plan into prds/",
        hint: archiveTarget.disabledReason ?? "archive",
        disabledReason: archiveTarget.disabledReason,
        run: () => {
          if (archiveTarget.disabledReason || !currentProjectId || !archiveTarget.planId) return;
          onArchivePlan(currentProjectId, archiveTarget.planId);
        },
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
    const removeProjectItems: Item[] = projects.map((p) => ({
      id: `proj-remove:${p.id}`,
      group: "Projects",
      title: `Remove ${repoName(p.repo_path)}…`,
      subtitle: p.repo_path,
      hint: "remove",
      icon: TrashIcon,
      run: () => onRemoveProject(p.id),
    }));
    const taskItems: Item[] = tasks.map((t) => ({
      id: `task:${t.projectId}:${t.planId}:${t.anchor}`,
      group: "Tasks",
      title: taskSummary(t.text),
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
      title: taskSummary(r.taskText),
      subtitle: `${RUN_STATUS_LABEL[r.status]} · ${r.agent} · ${r.projectName}`,
      hint: r.status,
      run: () => onOpenRun(r.runId),
    }));
    return [...actions, ...projectItems, ...removeProjectItems, ...taskItems, ...runItems];
  }, [
    projects,
    tasks,
    runs,
    archiveTarget,
    currentProjectId,
    onAddProject,
    onOpenOverview,
    onArchivePlan,
    onOpenProject,
    onRemoveProject,
    onOpenTask,
    onOpenRun,
  ]);

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
      if (item && !item.disabledReason) {
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
    <div
      className="palette__overlay"
      role="dialog"
      aria-modal="true"
      aria-label="Command palette"
      onMouseDown={(e) => {
        // Click on the dimmed backdrop (not the panel) dismisses — the standard
        // command-palette gesture, alongside Esc.
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="palette">
        <div className="palette__input-wrap">
          <SearchIcon className="palette__input-icon" />
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
        </div>
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
                  const disabled = item.disabledReason !== undefined;
                  const GroupIcon = item.icon ?? GROUP_ICON[item.group];
                  return (
                    <button
                      key={item.id}
                      data-idx={idx}
                      className={`palette__row${active ? " palette__row--active" : ""}${
                        disabled ? " palette__row--disabled" : ""
                      }`}
                      aria-current={active}
                      aria-disabled={disabled}
                      title={item.disabledReason}
                      onMouseEnter={() => setSelected(idx)}
                      onClick={() => {
                        if (disabled) return;
                        item.run();
                        onClose();
                      }}
                    >
                      <GroupIcon className="palette__row-icon" />
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
          {NAV_HINTS.map((hint) => (
            <span key={hint.label}>
              {hint.keys.map((k) => (
                <kbd key={k}>{k}</kbd>
              ))}{" "}
              {hint.label}
            </span>
          ))}
          {SHORTCUTS.map((s) => (
            <span key={s.id}>
              {shortcutKeyGlyphs(s).map((g, i) => (
                <kbd key={i}>{g}</kbd>
              ))}{" "}
              {s.label}
            </span>
          ))}
        </div>
      </div>
    </div>
  );
}
