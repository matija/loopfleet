import { useEffect, useState } from "react";
import { listProjects } from "./commands";
import { onRunStatus } from "./events";
import type { Project } from "./types";

// Scaffold surface: proves the React app boots inside the Tauri WebView and
// reaches the unchanged Rust command surface through the typed wrappers
// (commands.ts / events.ts / types.ts), one command + one live event stream. The
// real UI (design system, views) lands in the following M7 tasks.
export default function App() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [lastStatus, setLastStatus] = useState<string | null>(null);

  useEffect(() => {
    listProjects()
      .then(setProjects)
      .catch((e) => setError(String(e)));

    const unlisten = onRunStatus((p) =>
      setLastStatus(`${p.run_id}: ${p.status}`),
    );
    return () => {
      unlisten.then((off) => off());
    };
  }, []);

  return (
    <main style={{ fontFamily: "system-ui, sans-serif", padding: "1.5rem" }}>
      <h1>loopfleet</h1>
      {error && <p style={{ color: "crimson" }}>{error}</p>}
      {projects.length === 0 ? (
        <p>No projects registered.</p>
      ) : (
        <ul>
          {projects.map((p) => (
            <li key={p.id}>
              {p.repo_path} <small>({p.plan_convention})</small>
            </li>
          ))}
        </ul>
      )}
      {lastStatus && <p><small>last run status — {lastStatus}</small></p>}
    </main>
  );
}
