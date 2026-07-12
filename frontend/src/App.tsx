import { useEffect, useState } from "react";
import { listProjects } from "./commands";
import type { Project } from "./types";
import { AppShell } from "./components/AppShell";
import { SandboxBoundaryPanel } from "./components/SandboxBoundaryPanel";

// Composition root for the shell. Loads the registered projects into the
// sidebar and drives which one the main pane is scoped to. The main pane is
// intentionally minimal here — the plan view, run surfaces and settings render
// into it in the following M7 tasks. What must be present now is the honest
// sandbox-boundary panel (a trust feature), visible on the overview.
export default function App() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    listProjects()
      .then((ps) => {
        setProjects(ps);
        setSelectedId((cur) => cur ?? ps[0]?.id ?? null);
      })
      .catch((e) => setError(String(e)));
  }, []);

  const selected = projects.find((p) => p.id === selectedId) ?? null;

  return (
    <AppShell
      sidebar={
        <>
          <div className="sidebar__section-label">Projects</div>
          {projects.length === 0 ? (
            <div className="sidebar__empty">
              No projects yet. Registering a project lands in the next view.
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
        <p className="main__placeholder">
          The plan view, live runs and compare surface land in the next M7
          tasks. Every run spawns under the boundary below.
        </p>
        <div style={{ marginTop: "var(--space-5)" }}>
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
