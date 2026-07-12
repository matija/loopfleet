import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

// Minimal scaffold surface: proves the React app boots inside the Tauri WebView
// and can call the unchanged Rust command surface via @tauri-apps/api (no
// withGlobalTauri). The real UI (design system, types, views) lands in the
// following M7 tasks.
type Project = {
  id: string;
  repo_path: string;
  plan_convention: string;
};

export default function App() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<Project[]>("list_projects")
      .then(setProjects)
      .catch((e) => setError(String(e)));
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
    </main>
  );
}
