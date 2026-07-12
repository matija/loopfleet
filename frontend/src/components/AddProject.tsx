// The add-project affordance: a native folder picker (Tauri dialog plugin, the
// same one the legacy UI used and already registered on the Rust side) → the
// unchanged `register_project` command. Registration errors (not a git repo,
// already registered) surface inline — the Rust core returns them as strings.

import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { registerProject } from "../commands";
import type { Project } from "../types";

export function AddProject({ onAdded }: { onAdded: (p: Project) => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function pick() {
    setError(null);
    let folder: string | null;
    try {
      folder = (await open({ directory: true, multiple: false })) as
        | string
        | null;
    } catch (e) {
      setError(String(e));
      return;
    }
    if (!folder) return; // picker cancelled
    setBusy(true);
    try {
      onAdded(await registerProject(folder));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="add-project">
      <button className="btn btn--accent" onClick={pick} disabled={busy}>
        {busy ? "Adding…" : "Add project…"}
      </button>
      {error && <div className="add-project__error">{error}</div>}
    </div>
  );
}
