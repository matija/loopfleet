import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ComponentType,
} from "react";
import {
  acknowledgeRuns,
  cancelScheduledResume,
  launchRun,
  listProjects,
  stopRun,
} from "./commands";
import { normalizeDisplayText } from "./displayText";
import { onRunStatus, onScheduledResume, onScheduledResumeCancelled } from "./events";
import type { Project } from "./types";
import { isActiveRun } from "./status";
import { AppShell } from "./components/AppShell";
import { AddProject, pickAndRegisterProject } from "./components/AddProject";
import { AgentStatusPanel } from "./components/AgentStatusPanel";
import { SettingsPanel } from "./components/SettingsPanel";
import { SandboxBoundaryPanel } from "./components/SandboxBoundaryPanel";
import { SurfaceCard, SurfaceCardGrid } from "./components/SurfaceCard";
import type { CompareTarget, LaunchedRun } from "./components/PlanView";
import { PlanSurface } from "./components/PlanSurface";
import { PlanTree } from "./components/PlanTree";
import { TaskTab } from "./components/TaskTab";
import { RunDock, type ActiveRun } from "./components/RunDock";
import { LiveRunView } from "./components/LiveRunView";
import { RunTimeline } from "./components/RunTimeline";
import { CompareView } from "./components/CompareView";
import { Toasts, useToasts } from "./components/Toasts";
import { Toolbar } from "./components/Toolbar";
import { IconButton } from "./components/Button";
import {
  AgentIcon,
  BoxIcon,
  ChevronRightIcon,
  DiffIcon,
  FolderIcon,
  PanelBottomIcon,
  PanelLeftIcon,
  PlayIcon,
  SettingsIcon,
  type IconProps,
} from "./components/Icon";
import {
  CommandPalette,
  type PaletteOpenTask,
} from "./components/CommandPalette";
import { useSidebarCollapsed } from "./sidebarCollapse";

const SIDEBAR_HIDDEN_KEY = "loopfleet.sidebar.hidden";
const DOCK_COLLAPSED_KEY = "loopfleet.dock.collapsed";

function readSidebarHidden(): boolean {
  try {
    return localStorage.getItem(SIDEBAR_HIDDEN_KEY) === "true";
  } catch {
    return false;
  }
}

function readDockCollapsed(): boolean {
  try {
    return localStorage.getItem(DOCK_COLLAPSED_KEY) === "true";
  } catch {
    return false;
  }
}

// A run streams live while active; once terminal, its persisted timeline (with
// per-iteration events and diffs) is the surface. Opening a run from the dock
// picks the right one by status, and a still-open live view flips to the
// timeline automatically when the run ends (`isActiveRun`, from status.ts).

// --- View model ------------------------------------------------------------
//
// The main pane shows exactly one view at a time, driven by a single `view`
// state. Selecting a project opens its plan; opening a task / run / compare
// replaces the current view; the in-view "← Back" control returns to the
// selected project's plan (or the overview when no project is selected). The
// sidebar's plan tree and the bottom run dock are the always-present navigators.

type View =
  | { kind: "overview" }
  | { kind: "plan"; projectId: string }
  | {
      kind: "task";
      projectId: string;
      planId: string;
      taskAnchor: string;
      taskText: string;
    }
  | { kind: "run"; runId: string }
  | { kind: "compare"; planId: string; taskAnchor: string; taskText: string };

// Composition root. Loads registered projects into the sidebar (connections
// analog) and hosts a single main pane whose content follows `view`. The dock
// spans the bottom as the global run surface.
export default function App() {
  const [projects, setProjects] = useState<Project[]>([]);
  // The connections list is itself an async surface: until the first
  // `listProjects` resolves, an empty `projects` must read as "loading", not as
  // the "No projects yet" empty state — and a load failure must surface, not be
  // masked by that same empty state. `projectsLoaded`/`projectsError` split the
  // three, mirroring the loaded+error pattern every other panel uses.
  const [projectsLoaded, setProjectsLoaded] = useState(false);
  const [projectsError, setProjectsError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [view, setView] = useState<View>({ kind: "overview" });
  // Live "filter tables…"-style narrowing of the connections list.
  const [projectFilter, setProjectFilter] = useState("");
  // The sidebar search row shows a "Search" label until clicked/focused, then
  // swaps to the live input; it reverts once blurred with no text entered.
  const [projectSearchActive, setProjectSearchActive] = useState(false);
  // App-level command errors surface as transient toasts, not a persistent
  // banner. Contextual form errors stay inline in their own components.
  const { toasts, push: pushError, dismiss: dismissToast } = useToasts();
  // Session-scoped registry of launched runs (the global run surface). Runs do
  // not survive a restart in v1, so this is complete for the session.
  const [runs, setRuns] = useState<ActiveRun[]>([]);
  // Bumped to force the plan overview to refetch after a run is accepted (its
  // derived TaskStatus changes).
  const [planNonce, setPlanNonce] = useState(0);
  // ⌘K command palette — global keyboard-first navigator across projects,
  // tasks, runs, and quick actions. Toggled by Cmd/Ctrl-K anywhere.
  const [paletteOpen, setPaletteOpen] = useState(false);
  // Whole-sidebar visibility — separate from the "Projects" disclosure above.
  // Toggled by the panel-left button in the sidebar top strip or ⌘B, and
  // persisted so the collapsed layout survives a reload.
  const [sidebarHidden, setSidebarHidden] = useState(readSidebarHidden);
  // Collapsed state of the run dock — separate from the sidebar toggle above.
  // Toggled by the panel-bottom button in the toolbar's trailing slot, and
  // persisted so the collapsed dock survives a reload.
  const [dockCollapsed, setDockCollapsed] = useState(readDockCollapsed);
  // Collapsed state of the "Projects" section disclosure, persisted so a
  // reload keeps the sidebar the way it was left.
  const [projectsCollapsed, toggleProjectsCollapsed] =
    useSidebarCollapsed("projects");
  // The toolbar's action-slot DOM node (Toolbar's ref), threaded down to
  // whichever view is open so its primary action button(s) can portal there —
  // Run/Compare for a task, Stop for a live run, Use run/Retry for a finished
  // run, Accept for a compare. Starts null until Toolbar's first commit.
  const [toolbarActionsEl, setToolbarActionsEl] =
    useState<HTMLDivElement | null>(null);
  // The toolbar's filter-slot DOM node, threaded down to LiveRunView so its
  // task pill / event filter / freshness / agent pill can portal there —
  // folding that per-tab strip into the shell's single toolbar. Starts null
  // until Toolbar's first commit.
  const [toolbarFilterEl, setToolbarFilterEl] =
    useState<HTMLDivElement | null>(null);

  useEffect(() => {
    listProjects()
      .then((ps) => {
        setProjects(ps);
        setProjectsLoaded(true);
        setSelectedId((cur) => {
          const next = cur ?? ps[0]?.id ?? null;
          if (next) setView({ kind: "plan", projectId: next });
          return next;
        });
      })
      .catch((e) => {
        setProjectsError(String(e));
        setProjectsLoaded(true);
      });
  }, []);

  // The run currently shown in the main pane, read live from event handlers
  // (below) without re-subscribing. A run finishing while it is the open view is
  // already seen, so it never gets flagged for attention.
  const openRunIdRef = useRef<string | null>(null);

  // Terminal-state updates for any run flow through the dock's registry. A run
  // reaching a terminal status while it is not the open view is flagged
  // `unseen` — the dock's attention marker — until acknowledged (see below).
  useEffect(() => {
    const un = onRunStatus((p) =>
      setRuns((prev) =>
        prev.map((r) =>
          r.runId === p.run_id
            ? {
                ...r,
                status: p.status,
                unseen:
                  !isActiveRun(p.status) &&
                  openRunIdRef.current !== p.run_id,
              }
            : r,
        ),
      ),
    );
    return () => {
      un.then((f) => f());
    };
  }, []);

  // Client-side timers that clear a run's `pendingResume` marker once its
  // resume actually fires — there is no backend "fired" event, only the
  // `resume_at` the schedule was set for, so we clear locally when that
  // instant passes. Keyed by run id so a cancel (below) can also clear the
  // matching timer instead of leaving it to fire on a since-cleared run.
  const resumeTimersRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(
    new Map(),
  );
  useEffect(() => {
    return () => {
      for (const t of resumeTimersRef.current.values()) clearTimeout(t);
    };
  }, []);

  // A rate-limited run's re-run has been scheduled: surface "resuming at…" on
  // its dock chip until the resume fires or is cancelled.
  useEffect(() => {
    const un = onScheduledResume((p) => {
      const resumeAtMs = new Date(p.resume_at).getTime();
      setRuns((prev) =>
        prev.map((r) =>
          r.runId === p.run_id ? { ...r, pendingResume: { resumeAt: resumeAtMs } } : r,
        ),
      );
      const existing = resumeTimersRef.current.get(p.run_id);
      if (existing) clearTimeout(existing);
      const delay = Math.max(0, resumeAtMs - Date.now());
      const timer = setTimeout(() => {
        resumeTimersRef.current.delete(p.run_id);
        setRuns((prev) =>
          prev.map((r) =>
            r.runId === p.run_id ? { ...r, pendingResume: undefined } : r,
          ),
        );
      }, delay);
      resumeTimersRef.current.set(p.run_id, timer);
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  // A scheduled re-run was cancelled before it fired: drop its "resuming at…"
  // marker and the local timer that would have cleared it anyway.
  useEffect(() => {
    const un = onScheduledResumeCancelled((p) => {
      const existing = resumeTimersRef.current.get(p.run_id);
      if (existing) {
        clearTimeout(existing);
        resumeTimersRef.current.delete(p.run_id);
      }
      setRuns((prev) =>
        prev.map((r) =>
          r.runId === p.run_id ? { ...r, pendingResume: undefined } : r,
        ),
      );
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  // Acknowledge on focus: returning to the app (its window regaining focus)
  // means the user is looking again, so clear every finished run's attention
  // marker. Opening a specific finished run acknowledges just that one (below).
  // The `some` guard keeps focus events that change nothing from re-rendering.
  // Also clears the OS-level dock badge/attention signal, since a JS `focus`
  // event doesn't necessarily coincide with the native window-focus transition
  // the backend otherwise relies on to clear it.
  useEffect(() => {
    function onFocus() {
      setRuns((prev) =>
        prev.some((r) => r.unseen)
          ? prev.map((r) => (r.unseen ? { ...r, unseen: false } : r))
          : prev,
      );
      acknowledgeRuns().catch(() => {});
    }
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

  // Global ⌘K / Ctrl-K toggles the command palette from anywhere. preventDefault
  // stops the browser's default "focus search" behavior on Ctrl-K.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((o) => !o);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Global ⌘B / Ctrl-B toggles the sidebar from anywhere, mirroring ⌘K above.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "b") {
        e.preventDefault();
        setSidebarHidden((h) => !h);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    try {
      localStorage.setItem(SIDEBAR_HIDDEN_KEY, String(sidebarHidden));
    } catch {
      // localStorage unavailable (private mode, quota) — the toggle just
      // doesn't persist across reloads.
    }
  }, [sidebarHidden]);

  useEffect(() => {
    try {
      localStorage.setItem(DOCK_COLLAPSED_KEY, String(dockCollapsed));
    } catch {
      // localStorage unavailable (private mode, quota) — the toggle just
      // doesn't persist across reloads.
    }
  }, [dockCollapsed]);

  const toggleSidebarHidden = useCallback(() => setSidebarHidden((h) => !h), []);
  const toggleDockCollapsed = useCallback(() => setDockCollapsed((c) => !c), []);

  const selected = projects.find((p) => p.id === selectedId) ?? null;
  // A connection's status dot lights while any of its runs is active. The dock
  // registry tags runs by project name (the only project handle it carries), so
  // the match is by repo name.
  const activeProjectNames = new Set(
    runs.filter((r) => isActiveRun(r.status)).map((r) => r.projectName),
  );
  const q = projectFilter.trim().toLowerCase();
  // The filter narrows both projects (by path) and, within the open connection,
  // its tasks (in PlanTree). The selected project stays pinned so its tree keeps
  // filtering tasks even when the query doesn't match its path.
  const visibleProjects = q
    ? projects.filter(
        (p) => p.id === selectedId || p.repo_path.toLowerCase().includes(q),
      )
    : projects;
  // The dock highlights whichever run is currently shown in the main pane.
  const selectedRunId = view.kind === "run" ? view.runId : null;
  // Keep the ref the run-status handler reads in sync with the open view.
  openRunIdRef.current = selectedRunId;

  // Open a run in the main pane and acknowledge it — opening a finished run is
  // the per-run counterpart to acknowledge-on-focus, clearing its attention
  // marker (and, since opening a run implies the window is focused, the same
  // OS-level dock badge/attention signal). Shared by the dock and the ⌘K palette.
  const openRun = useCallback((runId: string) => {
    setView({ kind: "run", runId });
    setRuns((prev) =>
      prev.map((r) => (r.runId === runId && r.unseen ? { ...r, unseen: false } : r)),
    );
    acknowledgeRuns().catch(() => {});
  }, []);

  // Return to the selected project's plan, or the overview when nothing is
  // selected. Used by the in-view "← Back" controls.
  const goBack = useCallback(() => {
    setView((cur) => {
      // Already on a plan/overview — nothing to go back to.
      if (cur.kind === "plan" || cur.kind === "overview") return cur;
      return selectedId
        ? { kind: "plan", projectId: selectedId }
        : { kind: "overview" };
    });
  }, [selectedId]);

  // A launched run joins the dock, tagged with the project it ran against.
  const onLaunch = useCallback(
    (run: LaunchedRun) => {
      const projectName = selected ? repoName(selected.repo_path) : "project";
      setRuns((prev) => [
        {
          runId: run.runId,
          projectName,
          taskText: run.taskText,
          agent: run.agent,
          model: run.model,
          maxIterations: run.maxIterations,
          startedAt: Date.now(),
          status: "running",
          projectId: selected?.id,
          taskAnchor: run.taskAnchor,
        },
        ...prev,
      ]);
      setDockCollapsed(false);
    },
    [selected],
  );

  // Retry a finished run: re-launch the same task with the same agent + passes.
  // The control behind a rate-limited run's "Retry now" — its project/task
  // identity rides the dock entry, so this works from the run view without a
  // plan open. The fresh run joins the dock and opens as the live view.
  const onRetry = useCallback(
    async (run: ActiveRun) => {
      if (!run.projectId || !run.taskAnchor) return;
      const maxIterations = run.maxIterations ?? 1;
      try {
        const runId = await launchRun({
          projectId: run.projectId,
          taskAnchor: run.taskAnchor,
          agent: run.agent,
          model: run.model,
          maxIterations,
        });
        setRuns((prev) => [
          {
            runId,
            projectName: run.projectName,
            taskText: run.taskText,
            agent: run.agent,
            model: run.model,
            maxIterations,
            startedAt: Date.now(),
            status: "running",
            projectId: run.projectId,
            taskAnchor: run.taskAnchor,
          },
          ...prev,
        ]);
        setDockCollapsed(false);
        openRun(runId);
      } catch (e) {
        pushError(String(e));
      }
    },
    [openRun, pushError],
  );

  // Selecting a project opens its plan in the main pane.
  function selectProject(id: string) {
    setSelectedId(id);
    setView({ kind: "plan", projectId: id });
  }

  // A newly registered project joins the list, becomes the selection, and opens
  // its plan.
  function onAdded(p: Project) {
    setProjects((prev) =>
      prev.some((x) => x.id === p.id) ? prev : [...prev, p],
    );
    setSelectedId(p.id);
    setView({ kind: "plan", projectId: p.id });
  }

  // ⌘K palette actions. Each opens the match in the main pane and closes the
  // palette. Opening a task also selects its project so the sidebar tree stays
  // in sync. The "add project" action reuses the shared pick-and-register flow.
  const paletteOpenProject = useCallback(
    (id: string) => selectProject(id),
    [],
  );
  const paletteOpenTask = useCallback((t: PaletteOpenTask) => {
    setSelectedId(t.projectId);
    setView({
      kind: "task",
      projectId: t.projectId,
      planId: t.planId,
      taskAnchor: t.taskAnchor,
      taskText: t.taskText,
    });
  }, []);
  const paletteOpenRun = useCallback(
    (runId: string) => openRun(runId),
    [openRun],
  );
  const paletteAddProject = useCallback(() => {
    pickAndRegisterProject()
      .then((p) => {
        if (p) onAdded(p);
      })
      .catch((e) => pushError(String(e)));
  }, [pushError]);
  const paletteOpenOverview = useCallback(
    () => setView({ kind: "overview" }),
    [],
  );

  return (
    <AppShell
      onOpenSettings={paletteOpenOverview}
      sidebarHidden={sidebarHidden}
      onToggleSidebar={toggleSidebarHidden}
      notice={null}
      titlebarTrailing={
        <button
          className="titlebar__k"
          onClick={() => setPaletteOpen(true)}
          title="Command palette"
          aria-label="Open command palette"
        >
          <span>Search</span>
          <kbd>⌘K</kbd>
        </button>
      }
      dock={
        <RunDock
          runs={runs}
          selectedRunId={selectedRunId}
          collapsed={dockCollapsed}
          onOpen={openRun}
          onStop={(id) => {
            stopRun(id).catch((e) => pushError(String(e)));
          }}
          onDismiss={(id) => {
            setRuns((prev) => prev.filter((r) => r.runId !== id));
            if (view.kind === "run" && view.runId === id) goBack();
          }}
          onCancelResume={(id) => {
            cancelScheduledResume(id).catch((e) => pushError(String(e)));
          }}
        />
      }
      sidebar={
        <>
          <div className="sidebar__section-head">
            <button
              type="button"
              className="sidebar__section-label sidebar__section-label--disclosure"
              aria-expanded={!projectsCollapsed}
              onClick={toggleProjectsCollapsed}
            >
              <ChevronRightIcon size={12} className="disclosure__chevron" />
              Projects
            </button>
            <AddProject onAdded={onAdded} compact />
          </div>
          {!projectsCollapsed && projects.length > 0 && (
            <div className="sidebar__search">
              <svg
                className="sidebar__search-icon"
                viewBox="0 0 16 16"
                fill="none"
                aria-hidden="true"
              >
                <circle
                  cx="7"
                  cy="7"
                  r="4.5"
                  stroke="currentColor"
                  strokeWidth="1.5"
                />
                <path
                  d="M10.5 10.5L14 14"
                  stroke="currentColor"
                  strokeWidth="1.5"
                  strokeLinecap="round"
                />
              </svg>
              {projectSearchActive || projectFilter ? (
                <input
                  className="sidebar__search-input"
                  type="text"
                  autoFocus
                  placeholder="Filter projects…"
                  aria-label="Filter projects"
                  value={projectFilter}
                  onChange={(e) => setProjectFilter(e.target.value)}
                  onBlur={() => {
                    if (!projectFilter) setProjectSearchActive(false);
                  }}
                />
              ) : (
                <button
                  type="button"
                  className="sidebar__search-label"
                  onClick={() => setProjectSearchActive(true)}
                  onFocus={() => setProjectSearchActive(true)}
                >
                  Search
                </button>
              )}
              <kbd className="sidebar__search-kbd">⌘K</kbd>
            </div>
          )}
          {projectsCollapsed ? null : !projectsLoaded ? (
            <div className="sidebar__empty">Loading projects…</div>
          ) : projectsError ? (
            <div className="sidebar__empty sidebar__empty--error">
              Couldn’t load projects: {projectsError}
            </div>
          ) : projects.length === 0 ? (
            <div className="sidebar__empty">
              No projects yet. Add a git repo to launch runs against its plan.
            </div>
          ) : visibleProjects.length === 0 ? (
            <div className="sidebar__empty">
              No projects match “{projectFilter.trim()}”.
            </div>
          ) : (
            <div className="sidebar__list">
              {visibleProjects.map((p) => (
                <div key={p.id}>
                  <button
                    className="project-item"
                    aria-current={p.id === selectedId}
                    onClick={() => selectProject(p.id)}
                  >
                    <FolderIcon size={16} className="project-item__icon" />
                    <span
                      className={`project-item__dot${
                        activeProjectNames.has(repoName(p.repo_path))
                          ? " project-item__dot--active"
                          : ""
                      }`}
                    />
                    <span className="project-item__name">
                      {repoName(p.repo_path)}
                    </span>
                    <span className="project-item__meta">
                      {parentPath(p.repo_path)}
                    </span>
                  </button>
                  {p.id === selectedId && (
                    <PlanTree
                      projectId={p.id}
                      filter={projectFilter}
                      nonce={planNonce}
                      activeTaskId={
                        view.kind === "task"
                          ? `task:${view.planId}:${view.taskAnchor}`
                          : null
                      }
                      onOpenTask={(t) =>
                        setView({
                          kind: "task",
                          projectId: p.id,
                          planId: t.planId,
                          taskAnchor: t.taskAnchor,
                          taskText: t.taskText,
                        })
                      }
                    />
                  )}
                </div>
              ))}
            </div>
          )}
        </>
      }
    >
      <Toasts toasts={toasts} onDismiss={dismissToast} />
      <Toolbar
        filterRef={setToolbarFilterEl}
        actionsRef={setToolbarActionsEl}
        breadcrumb={<Breadcrumb crumbs={crumbsFor(view, projects, runs, selectProject)} />}
        trailing={
          <>
            <IconButton
              icon={PanelLeftIcon}
              aria-label="Sidebar"
              aria-pressed={!sidebarHidden}
              onClick={toggleSidebarHidden}
              title="Toggle sidebar (⌘B)"
            />
            <IconButton
              icon={PanelBottomIcon}
              aria-label="Run dock"
              aria-pressed={!dockCollapsed}
              onClick={toggleDockCollapsed}
              title="Toggle run dock"
            />
          </>
        }
      />
      <div
        className={`main__body${
          view.kind === "run" || view.kind === "compare"
            ? " main__body--run"
            : ""
        }`}
      >
        {view.kind === "run" ? (
          <RunPane
            runId={view.runId}
            runs={runs}
            onStop={(id) => {
              stopRun(id).catch((e) => pushError(String(e)));
            }}
            onClose={goBack}
            onAccepted={() => setPlanNonce((n) => n + 1)}
            onRetry={onRetry}
            toolbarActions={toolbarActionsEl}
            toolbarFilter={toolbarFilterEl}
          />
        ) : view.kind === "compare" ? (
          <CompareView
            key={`compare:${view.planId}:${view.taskAnchor}`}
            planId={view.planId}
            taskAnchor={view.taskAnchor}
            taskText={view.taskText}
            onClose={goBack}
            onAccepted={() => setPlanNonce((n) => n + 1)}
            toolbarActions={toolbarActionsEl}
          />
        ) : view.kind === "task" ? (
          <TaskTab
            key={`task:${view.planId}:${view.taskAnchor}`}
            projectId={view.projectId}
            planId={view.planId}
            taskAnchor={view.taskAnchor}
            nonce={planNonce}
            onLaunch={onLaunch}
            onLaunched={() => setPlanNonce((n) => n + 1)}
            onError={pushError}
            onCompare={(target: CompareTarget) =>
              setView({
                kind: "compare",
                planId: target.planId,
                taskAnchor: target.taskAnchor,
                taskText: target.taskText,
              })
            }
            toolbarActions={toolbarActionsEl}
          />
        ) : view.kind === "plan" ? (
          <PlanSurface
            projectId={view.projectId}
            planNonce={planNonce}
            onLaunch={onLaunch}
            onError={pushError}
            onCompare={(target: CompareTarget) =>
              setView({
                kind: "compare",
                planId: target.planId,
                taskAnchor: target.taskAnchor,
                taskText: target.taskText,
              })
            }
            toolbarActions={toolbarActionsEl}
          />
        ) : (
          <Overview />
        )}
      </div>
      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        projects={projects}
        runs={runs}
        onOpenProject={paletteOpenProject}
        onOpenTask={paletteOpenTask}
        onOpenRun={paletteOpenRun}
        onAddProject={paletteAddProject}
        onOpenOverview={paletteOpenOverview}
      />
    </AppShell>
  );
}

// The overview's three machine-level surfaces, each an entry-point tile.
type OverviewSection = "agents" | "defaults" | "sandbox";

// The no-project main pane: a surface-card grid over the machine-level panels.
// Each card toggles its panel open beneath the grid — one section at a time,
// clicking the open card's tile again collapses it. The panels themselves
// (agent chips, settings form, sandbox rules) are unchanged; the cards only
// gate when they mount.
function Overview() {
  const [expanded, setExpanded] = useState<OverviewSection | null>(null);

  const toggle = (section: OverviewSection) =>
    setExpanded((cur) => (cur === section ? null : section));

  return (
    <div className="overview">
      <SurfaceCardGrid>
        <SurfaceCard
          icon={<AgentIcon />}
          title="Agents"
          description="Agent CLIs on this machine, with version drift"
          onClick={() => toggle("agents")}
        />
        <SurfaceCard
          icon={<SettingsIcon />}
          title="Run defaults"
          description="Default agent, iteration count, concurrency cap"
          onClick={() => toggle("defaults")}
        />
        <SurfaceCard
          icon={<BoxIcon />}
          title="Sandbox boundary"
          description="What every run's OS sandbox does and doesn't confine"
          onClick={() => toggle("sandbox")}
        />
      </SurfaceCardGrid>
      {expanded === "agents" ? (
        <AgentStatusPanel />
      ) : expanded === "defaults" ? (
        <SettingsPanel />
      ) : expanded === "sandbox" ? (
        <SandboxBoundaryPanel />
      ) : null}
    </div>
  );
}

// The run pane hosts the live view while the run is active, then flips to the
// persisted timeline once terminal. The run is looked up from the session
// registry by id; a dismissed run navigates back, so a miss is only a transient
// race and renders a quiet fallback.
function RunPane({
  runId,
  runs,
  onStop,
  onClose,
  onAccepted,
  onRetry,
  toolbarActions,
  toolbarFilter,
}: {
  runId: string;
  runs: ActiveRun[];
  onStop: (runId: string) => void;
  onClose: () => void;
  onAccepted: () => void;
  onRetry: (run: ActiveRun) => void;
  toolbarActions: HTMLDivElement | null;
  toolbarFilter: HTMLDivElement | null;
}) {
  const run = runs.find((r) => r.runId === runId);
  if (!run) {
    return <p className="main__placeholder">This run is no longer available.</p>;
  }
  return isActiveRun(run.status) ? (
    <LiveRunView
      key={run.runId}
      run={run}
      onStop={onStop}
      onClose={onClose}
      toolbarActions={toolbarActions}
      toolbarFilter={toolbarFilter}
    />
  ) : (
    <RunTimeline
      key={run.runId}
      run={run}
      onClose={onClose}
      onAccepted={onAccepted}
      onRetry={onRetry}
      toolbarActions={toolbarActions}
    />
  );
}

// One breadcrumb segment in the toolbar: project → plan → task/run/compare.
// `onClick` is only set when the caller passes `onSelectProject` (App wires
// its `selectProject`); without it every segment renders as inert text.
export type Crumb = {
  label: string;
  icon?: ComponentType<IconProps>;
  onClick?: () => void;
};

// Pure breadcrumb model for the active view. Runs carry their own project
// name/id (set at launch, see `onLaunch`), which is how the `run` case finds
// its project without a projects lookup. `compare` has no projectId on the
// view itself, so it best-effort recovers one from a run sharing its task
// anchor — the compare flow is only reachable from a task with existing runs.
export function crumbsFor(
  view: View,
  projects: Project[],
  runs: ActiveRun[],
  onSelectProject?: (projectId: string) => void,
): Crumb[] {
  function projectCrumb(projectId: string | undefined, label: string): Crumb {
    return {
      label,
      icon: FolderIcon,
      onClick:
        projectId && onSelectProject
          ? () => onSelectProject(projectId)
          : undefined,
    };
  }

  function planCrumb(projectId: string | undefined): Crumb {
    return {
      label: "Plan",
      onClick:
        projectId && onSelectProject
          ? () => onSelectProject(projectId)
          : undefined,
    };
  }

  switch (view.kind) {
    case "overview":
      return [{ label: "Overview" }];
    case "plan": {
      const p = projects.find((x) => x.id === view.projectId);
      return [
        projectCrumb(view.projectId, p ? repoName(p.repo_path) : "Project"),
        { label: "Plan" },
      ];
    }
    case "task": {
      const p = projects.find((x) => x.id === view.projectId);
      return [
        projectCrumb(view.projectId, p ? repoName(p.repo_path) : "Project"),
        planCrumb(view.projectId),
        { label: normalizeDisplayText(view.taskText) },
      ];
    }
    case "run": {
      const r = runs.find((x) => x.runId === view.runId);
      if (!r) return [{ label: "Run", icon: PlayIcon }];
      return [
        projectCrumb(r.projectId, r.projectName),
        planCrumb(r.projectId),
        { label: normalizeDisplayText(r.taskText), icon: PlayIcon },
      ];
    }
    case "compare": {
      const r = runs.find((x) => x.taskAnchor === view.taskAnchor);
      const crumbs: Crumb[] = [];
      if (r) {
        crumbs.push(projectCrumb(r.projectId, r.projectName));
        crumbs.push(planCrumb(r.projectId));
      }
      crumbs.push({
        label: normalizeDisplayText(view.taskText),
        icon: DiffIcon,
      });
      return crumbs;
    }
  }
}

// Renders a Crumb[] as `--text-base` muted segments joined by dim "/"
// separators, with the final (current) segment in `--c-text`. Non-final
// segments with an `onClick` are buttons; everything else is inert text.
function Breadcrumb({ crumbs }: { crumbs: Crumb[] }) {
  return (
    <>
      {crumbs.map((c, i) => {
        const isLast = i === crumbs.length - 1;
        const Icon = c.icon;
        const content = (
          <>
            {Icon && <Icon size={13} className="toolbar__crumb-icon" />}
            {c.label}
          </>
        );
        return (
          <span className="toolbar__crumb-group" key={i}>
            {i > 0 && <span className="toolbar__crumb-sep">/</span>}
            {!isLast && c.onClick ? (
              <button type="button" className="toolbar__crumb" onClick={c.onClick}>
                {content}
              </button>
            ) : (
              <span
                className={`toolbar__crumb${isLast ? " toolbar__crumb--current" : ""}`}
              >
                {content}
              </span>
            )}
          </span>
        );
      })}
    </>
  );
}

// The trailing path segment — the connection row's title.
function repoName(path: string): string {
  const parts = path.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || path;
}

// Everything before the repo name — the connection row's subtitle (the DB
// client's `user@host` analog).
function parentPath(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  const idx = trimmed.lastIndexOf("/");
  return idx > 0 ? trimmed.slice(0, idx) : trimmed;
}
