import { useCallback, useEffect, useState } from "react";
import { listProjects, stopRun } from "./commands";
import { onRunStatus } from "./events";
import type { Project } from "./types";
import { AppShell } from "./components/AppShell";
import { AddProject } from "./components/AddProject";
import { AgentStatusPanel } from "./components/AgentStatusPanel";
import { SettingsPanel } from "./components/SettingsPanel";
import { SandboxOverrides } from "./components/SandboxOverrides";
import { SandboxBoundaryPanel } from "./components/SandboxBoundaryPanel";
import { PlanView, type LaunchedRun } from "./components/PlanView";
import { RunDock, type ActiveRun } from "./components/RunDock";

// Composition root for the shell. Loads registered projects into the sidebar
// (with the add-project affordance) and scopes the main pane to a selection.
// The main pane is the overview: agent availability, settings, the selected
// project's sandbox overrides, and the honest sandbox-boundary trust panel. The
// plan view and run surfaces render here in the following M7 tasks.
export default function App() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Session-scoped registry of launched runs (the global run surface). Runs do
  // not survive a restart in v1, so this is complete for the session.
  const [runs, setRuns] = useState<ActiveRun[]>([]);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);

  useEffect(() => {
    listProjects()
      .then((ps) => {
        setProjects(ps);
        setSelectedId((cur) => cur ?? ps[0]?.id ?? null);
      })
      .catch((e) => setError(String(e)));
  }, []);

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

  // A newly registered project joins the list and becomes the selection.
  function onAdded(p: Project) {
    setProjects((prev) =>
      prev.some((x) => x.id === p.id) ? prev : [...prev, p],
    );
    setSelectedId(p.id);
  }

  return (
    <AppShell
      dock={
        <RunDock
          runs={runs}
          selectedRunId={selectedRunId}
          onOpen={setSelectedRunId}
          onStop={(id) => {
            stopRun(id).catch((e) => setError(String(e)));
          }}
          onDismiss={(id) =>
            setRuns((prev) => prev.filter((r) => r.runId !== id))
          }
        />
      }
      sidebar={
        <>
          <div className="sidebar__section-label">Projects</div>
          <AddProject onAdded={onAdded} />
          {projects.length === 0 ? (
            <div className="sidebar__empty">
              No projects yet. Add a git repo to launch runs against its plan.
            </div>
          ) : (
            <div className="sidebar__list">
              {projects.map((p) => (
                <button
                  key={p.id}
                  className="project-item"
                  aria-current={p.id === selectedId}
                  onClick={() => setSelectedId(p.id)}
                >
                  <div className="project-item__name">{p.repo_path}</div>
                  <div className="project-item__meta">{p.plan_convention}</div>
                </button>
              ))}
            </div>
          )}
        </>
      }
    >
      {error && <div className="banner-error">{error}</div>}
      <div className="main__header">
        <h2>{selected ? repoName(selected.repo_path) : "Overview"}</h2>
        <p>
          {selected
            ? selected.repo_path
            : "Supervise looping coding agents in sandboxed git worktrees."}
        </p>
      </div>
      <div className="main__body">
        {selected ? (
          <PlanView
            key={selected.id}
            projectId={selected.id}
            onLaunch={onLaunch}
          />
        ) : (
          <p className="main__placeholder">
            Select or add a project to see its plan and launch runs.
          </p>
        )}
        <div className="overview">
          <AgentStatusPanel />
          <SettingsPanel />
          {selected && <SandboxOverrides projectId={selected.id} />}
          <SandboxBoundaryPanel />
        </div>
      </div>
    </AppShell>
  );
}

// The trailing path segment — the sidebar shows the full path, the header the
// short repo name.
function repoName(path: string): string {
  const parts = path.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || path;
}
