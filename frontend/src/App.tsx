import { useCallback, useEffect, useReducer, useState } from "react";
import { listProjects, stopRun } from "./commands";
import { onRunStatus } from "./events";
import type { Project, RunStatus } from "./types";
import { AppShell } from "./components/AppShell";
import { AddProject } from "./components/AddProject";
import { AgentStatusPanel } from "./components/AgentStatusPanel";
import { SettingsPanel } from "./components/SettingsPanel";
import { SandboxOverrides } from "./components/SandboxOverrides";
import { SandboxBoundaryPanel } from "./components/SandboxBoundaryPanel";
import {
  PlanView,
  type CompareTarget,
  type LaunchedRun,
} from "./components/PlanView";
import { PlanTree } from "./components/PlanTree";
import { TaskTab } from "./components/TaskTab";
import { RunDock, type ActiveRun } from "./components/RunDock";
import { LiveRunView } from "./components/LiveRunView";
import { RunTimeline } from "./components/RunTimeline";
import { CompareView } from "./components/CompareView";
import { TabStrip } from "./components/TabStrip";
import { Toasts, useToasts } from "./components/Toasts";

// A run streams live while active; once terminal, its persisted timeline (with
// per-iteration events and diffs) is the surface. Opening a run from the dock
// picks the right one by status, and a still-open live view flips to the
// timeline automatically when the run ends.
const ACTIVE: RunStatus[] = ["queued", "running"];

// --- Tab model ------------------------------------------------------------
//
// The workbench opens each task/run/compare as an independent, closeable tab
// instead of the M7 mutually-exclusive `selectedRun` / `compareTarget` switch
// (which could only ever show one thing). A pinned "Welcome" home is always the
// first tab and cannot be closed. The TabStrip styling lands in the next task;
// here the model + a functional tab bar wire the behavior.
type WorkbenchTab =
  | { kind: "welcome" }
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

// Stable identity per tab: opening the same object focuses its tab rather than
// stacking a duplicate.
function tabId(t: WorkbenchTab): string {
  switch (t.kind) {
    case "welcome":
      return "welcome";
    case "plan":
      return `plan:${t.projectId}`;
    case "task":
      return `task:${t.planId}:${t.taskAnchor}`;
    case "run":
      return `run:${t.runId}`;
    case "compare":
      return `compare:${t.planId}:${t.taskAnchor}`;
  }
}

type TabState = { tabs: WorkbenchTab[]; activeId: string };
type TabAction =
  | { type: "open"; tab: WorkbenchTab }
  | { type: "focus"; id: string }
  | { type: "close"; id: string };

function tabReducer(state: TabState, action: TabAction): TabState {
  switch (action.type) {
    case "focus":
      return { ...state, activeId: action.id };
    case "open": {
      const id = tabId(action.tab);
      const exists = state.tabs.some((t) => tabId(t) === id);
      return {
        tabs: exists ? state.tabs : [...state.tabs, action.tab],
        activeId: id,
      };
    }
    case "close": {
      const idx = state.tabs.findIndex((t) => tabId(t) === action.id);
      // idx <= 0 means not found or the pinned Welcome tab (always index 0).
      if (idx <= 0) return state;
      const tabs = state.tabs.filter((_, i) => i !== idx);
      // Closing the active tab falls back to its left neighbor (Welcome at
      // worst), which always exists since Welcome is pinned at index 0.
      const activeId =
        state.activeId === action.id
          ? tabId(state.tabs[idx - 1])
          : state.activeId;
      return { tabs, activeId };
    }
  }
}

// Composition root for the workbench. Loads registered projects into the
// sidebar (connections analog) and hosts a browser-style tab surface: the
// active tab drives the main pane, the dock spans the bottom.
export default function App() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // Live "filter tables…"-style narrowing of the connections list.
  const [projectFilter, setProjectFilter] = useState("");
  // App-level command errors surface as transient toasts, not a persistent
  // banner. Contextual form errors stay inline in their own components.
  const { toasts, push: pushError, dismiss: dismissToast } = useToasts();
  // Session-scoped registry of launched runs (the global run surface). Runs do
  // not survive a restart in v1, so this is complete for the session.
  const [runs, setRuns] = useState<ActiveRun[]>([]);
  const [tabState, dispatch] = useReducer(tabReducer, {
    tabs: [{ kind: "welcome" }],
    activeId: "welcome",
  });
  // Bumped to force the plan overview to refetch after a run is accepted (its
  // derived TaskStatus changes).
  const [planNonce, setPlanNonce] = useState(0);

  useEffect(() => {
    listProjects()
      .then((ps) => {
        setProjects(ps);
        setSelectedId((cur) => cur ?? ps[0]?.id ?? null);
      })
      .catch((e) => pushError(String(e)));
  }, [pushError]);

  // Terminal-state updates for any run flow through the dock's registry.
  useEffect(() => {
    const un = onRunStatus((p) =>
      setRuns((prev) =>
        prev.map((r) =>
          r.runId === p.run_id ? { ...r, status: p.status } : r,
        ),
      ),
    );
    return () => {
      un.then((f) => f());
    };
  }, []);

  const selected = projects.find((p) => p.id === selectedId) ?? null;
  // A connection's status dot lights while any of its runs is active. The dock
  // registry tags runs by project name (the only project handle it carries), so
  // the match is by repo name.
  const activeProjectNames = new Set(
    runs.filter((r) => ACTIVE.includes(r.status)).map((r) => r.projectName),
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
  const { tabs, activeId } = tabState;
  const activeTab = tabs.find((t) => tabId(t) === activeId) ?? tabs[0];
  // The dock highlights whichever run tab is currently active.
  const selectedRunId = activeTab.kind === "run" ? activeTab.runId : null;

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
          status: "running",
        },
        ...prev,
      ]);
    },
    [selected],
  );

  // A newly registered project joins the list, becomes the selection, and opens
  // its plan tab.
  function onAdded(p: Project) {
    setProjects((prev) =>
      prev.some((x) => x.id === p.id) ? prev : [...prev, p],
    );
    setSelectedId(p.id);
    dispatch({ type: "open", tab: { kind: "plan", projectId: p.id } });
  }

  return (
    <AppShell
      dock={
        <RunDock
          runs={runs}
          selectedRunId={selectedRunId}
          onOpen={(id) => dispatch({ type: "open", tab: { kind: "run", runId: id } })}
          onStop={(id) => {
            stopRun(id).catch((e) => pushError(String(e)));
          }}
          onDismiss={(id) => {
            setRuns((prev) => prev.filter((r) => r.runId !== id));
            dispatch({ type: "close", id: `run:${id}` });
          }}
        />
      }
      sidebar={
        <>
          <div className="sidebar__section-head">
            <div className="sidebar__section-label">Projects</div>
            <AddProject onAdded={onAdded} compact />
          </div>
          {projects.length > 0 && (
            <input
              className="sidebar__filter"
              type="text"
              placeholder="Filter projects…"
              aria-label="Filter projects"
              value={projectFilter}
              onChange={(e) => setProjectFilter(e.target.value)}
            />
          )}
          {projects.length === 0 ? (
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
                    onClick={() => {
                      setSelectedId(p.id);
                      dispatch({
                        type: "open",
                        tab: { kind: "plan", projectId: p.id },
                      });
                    }}
                  >
                    <span
                      className={`project-item__dot${
                        activeProjectNames.has(repoName(p.repo_path))
                          ? " project-item__dot--active"
                          : ""
                      }`}
                    />
                    <span className="project-item__body">
                      <span className="project-item__name">
                        {repoName(p.repo_path)}
                      </span>
                      <span className="project-item__meta">
                        {parentPath(p.repo_path)}
                      </span>
                    </span>
                  </button>
                  {p.id === selectedId && (
                    <PlanTree
                      projectId={p.id}
                      filter={projectFilter}
                      nonce={planNonce}
                      activeTaskId={
                        activeTab.kind === "task" ? tabId(activeTab) : null
                      }
                      onOpenTask={(t) =>
                        dispatch({
                          type: "open",
                          tab: {
                            kind: "task",
                            projectId: p.id,
                            planId: t.planId,
                            taskAnchor: t.taskAnchor,
                            taskText: t.taskText,
                          },
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
      <TabStrip
        tabs={tabs.map((t) => ({
          id: tabId(t),
          kind: t.kind,
          label: tabLabel(t, projects, runs),
        }))}
        activeId={activeId}
        onFocus={(id) => dispatch({ type: "focus", id })}
        onClose={(id) => dispatch({ type: "close", id })}
      />
      <div className="main__header">
        <h2>{headerFor(activeTab, projects, runs).title}</h2>
        <p>{headerFor(activeTab, projects, runs).subtitle}</p>
      </div>
      <div
        className={`main__body${
          activeTab.kind === "run" || activeTab.kind === "compare"
            ? " main__body--run"
            : ""
        }`}
      >
        {activeTab.kind === "run" ? (
          <RunTab
            runId={activeTab.runId}
            runs={runs}
            onStop={(id) => {
              stopRun(id).catch((e) => pushError(String(e)));
            }}
            onClose={() => dispatch({ type: "close", id: `run:${activeTab.runId}` })}
          />
        ) : activeTab.kind === "compare" ? (
          <CompareView
            key={tabId(activeTab)}
            planId={activeTab.planId}
            taskAnchor={activeTab.taskAnchor}
            taskText={activeTab.taskText}
            onClose={() => dispatch({ type: "close", id: tabId(activeTab) })}
            onAccepted={() => setPlanNonce((n) => n + 1)}
          />
        ) : activeTab.kind === "task" ? (
          <TaskTab
            key={tabId(activeTab)}
            projectId={activeTab.projectId}
            planId={activeTab.planId}
            taskAnchor={activeTab.taskAnchor}
            nonce={planNonce}
            onLaunch={onLaunch}
            onLaunched={() => setPlanNonce((n) => n + 1)}
            onCompare={(target: CompareTarget) =>
              dispatch({
                type: "open",
                tab: {
                  kind: "compare",
                  planId: target.planId,
                  taskAnchor: target.taskAnchor,
                  taskText: target.taskText,
                },
              })
            }
          />
        ) : activeTab.kind === "plan" ? (
          <>
            <PlanView
              key={`${activeTab.projectId}:${planNonce}`}
              projectId={activeTab.projectId}
              onLaunch={onLaunch}
              onCompare={(target: CompareTarget) =>
                dispatch({
                  type: "open",
                  tab: {
                    kind: "compare",
                    planId: target.planId,
                    taskAnchor: target.taskAnchor,
                    taskText: target.taskText,
                  },
                })
              }
            />
            <SandboxOverrides projectId={activeTab.projectId} />
          </>
        ) : (
          <>
            <p className="main__placeholder">
              Select or add a project to see its plan and launch runs.
            </p>
            <div className="overview">
              <AgentStatusPanel />
              <SettingsPanel />
              <SandboxBoundaryPanel />
            </div>
          </>
        )}
      </div>
    </AppShell>
  );
}

// A run tab hosts the live view while the run is active, then flips to the
// persisted timeline once terminal. The run is looked up from the session
// registry by id; a dismissed run closes its own tab, so a miss is only a
// transient race and renders a quiet fallback.
function RunTab({
  runId,
  runs,
  onStop,
  onClose,
}: {
  runId: string;
  runs: ActiveRun[];
  onStop: (runId: string) => void;
  onClose: () => void;
}) {
  const run = runs.find((r) => r.runId === runId);
  if (!run) {
    return <p className="main__placeholder">This run is no longer available.</p>;
  }
  return ACTIVE.includes(run.status) ? (
    <LiveRunView key={run.runId} run={run} onStop={onStop} onClose={onClose} />
  ) : (
    <RunTimeline key={run.runId} run={run} onClose={onClose} />
  );
}

// Short label shown on a tab.
function tabLabel(
  t: WorkbenchTab,
  projects: Project[],
  runs: ActiveRun[],
): string {
  switch (t.kind) {
    case "welcome":
      return "Welcome";
    case "plan": {
      const p = projects.find((x) => x.id === t.projectId);
      return p ? repoName(p.repo_path) : "Plan";
    }
    case "task":
      return truncate(t.taskText);
    case "run": {
      const r = runs.find((x) => x.runId === t.runId);
      return r ? truncate(r.taskText) : "Run";
    }
    case "compare":
      return `Compare · ${truncate(t.taskText)}`;
  }
}

// Header title/subtitle for the active tab's context.
function headerFor(
  t: WorkbenchTab,
  projects: Project[],
  runs: ActiveRun[],
): { title: string; subtitle: string } {
  switch (t.kind) {
    case "welcome":
      return {
        title: "Overview",
        subtitle:
          "Supervise looping coding agents in sandboxed git worktrees.",
      };
    case "plan": {
      const p = projects.find((x) => x.id === t.projectId);
      return {
        title: p ? repoName(p.repo_path) : "Plan",
        subtitle: p ? p.repo_path : "",
      };
    }
    case "task":
      return { title: "Task", subtitle: t.taskText };
    case "run": {
      const r = runs.find((x) => x.runId === t.runId);
      return { title: "Run", subtitle: r ? r.taskText : "" };
    }
    case "compare":
      return { title: "Compare", subtitle: t.taskText };
  }
}

function truncate(s: string, n = 32): string {
  return s.length > n ? `${s.slice(0, n - 1)}…` : s;
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
