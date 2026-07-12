import { useCallback, useEffect, useState } from "react";
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
import { Toasts, useToasts } from "./components/Toasts";

// A run streams live while active; once terminal, its persisted timeline (with
// per-iteration events and diffs) is the surface. Opening a run from the dock
// picks the right one by status, and a still-open live view flips to the
// timeline automatically when the run ends.
const ACTIVE: RunStatus[] = ["queued", "running"];

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
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [view, setView] = useState<View>({ kind: "overview" });
  // Live "filter tables…"-style narrowing of the connections list.
  const [projectFilter, setProjectFilter] = useState("");
  // App-level command errors surface as transient toasts, not a persistent
  // banner. Contextual form errors stay inline in their own components.
  const { toasts, push: pushError, dismiss: dismissToast } = useToasts();
  // Session-scoped registry of launched runs (the global run surface). Runs do
  // not survive a restart in v1, so this is complete for the session.
  const [runs, setRuns] = useState<ActiveRun[]>([]);
  // Bumped to force the plan overview to refetch after a run is accepted (its
  // derived TaskStatus changes).
  const [planNonce, setPlanNonce] = useState(0);

  useEffect(() => {
    listProjects()
      .then((ps) => {
        setProjects(ps);
        setSelectedId((cur) => {
          const next = cur ?? ps[0]?.id ?? null;
          if (next) setView({ kind: "plan", projectId: next });
          return next;
        });
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
  // The dock highlights whichever run is currently shown in the main pane.
  const selectedRunId = view.kind === "run" ? view.runId : null;

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
          status: "running",
        },
        ...prev,
      ]);
    },
    [selected],
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

  return (
    <AppShell
      dock={
        <RunDock
          runs={runs}
          selectedRunId={selectedRunId}
          onOpen={(id) => setView({ kind: "run", runId: id })}
          onStop={(id) => {
            stopRun(id).catch((e) => pushError(String(e)));
          }}
          onDismiss={(id) => {
            setRuns((prev) => prev.filter((r) => r.runId !== id));
            if (view.kind === "run" && view.runId === id) goBack();
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
                    onClick={() => selectProject(p.id)}
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
      <div className="main__header">
        <h2>{headerFor(view, projects, runs).title}</h2>
        <p>{headerFor(view, projects, runs).subtitle}</p>
      </div>
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
          />
        ) : view.kind === "compare" ? (
          <CompareView
            key={`compare:${view.planId}:${view.taskAnchor}`}
            planId={view.planId}
            taskAnchor={view.taskAnchor}
            taskText={view.taskText}
            onClose={goBack}
            onAccepted={() => setPlanNonce((n) => n + 1)}
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
            onCompare={(target: CompareTarget) =>
              setView({
                kind: "compare",
                planId: target.planId,
                taskAnchor: target.taskAnchor,
                taskText: target.taskText,
              })
            }
          />
        ) : view.kind === "plan" ? (
          <>
            <PlanView
              key={`${view.projectId}:${planNonce}`}
              projectId={view.projectId}
              onLaunch={onLaunch}
              onCompare={(target: CompareTarget) =>
                setView({
                  kind: "compare",
                  planId: target.planId,
                  taskAnchor: target.taskAnchor,
                  taskText: target.taskText,
                })
              }
            />
            <SandboxOverrides projectId={view.projectId} />
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

// The run pane hosts the live view while the run is active, then flips to the
// persisted timeline once terminal. The run is looked up from the session
// registry by id; a dismissed run navigates back, so a miss is only a transient
// race and renders a quiet fallback.
function RunPane({
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

// Header title/subtitle for the active view's context.
function headerFor(
  v: View,
  projects: Project[],
  runs: ActiveRun[],
): { title: string; subtitle: string } {
  switch (v.kind) {
    case "overview":
      return {
        title: "Overview",
        subtitle:
          "Supervise looping coding agents in sandboxed git worktrees.",
      };
    case "plan": {
      const p = projects.find((x) => x.id === v.projectId);
      return {
        title: p ? repoName(p.repo_path) : "Plan",
        subtitle: p ? p.repo_path : "",
      };
    }
    case "task":
      return { title: "Task", subtitle: v.taskText };
    case "run": {
      const r = runs.find((x) => x.runId === v.runId);
      return { title: "Run", subtitle: r ? r.taskText : "" };
    }
    case "compare":
      return { title: "Compare", subtitle: v.taskText };
  }
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
