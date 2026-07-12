// The add-project affordance: a native folder picker (Tauri dialog plugin, the
// same one the legacy UI used and already registered on the Rust side) → the
// unchanged `register_project` command. Registration errors (not a git repo,
// already registered) surface inline — the Rust core returns them as strings.

import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { registerProject } from "../commands";
import type { Project } from "../types";

/// The shared pick-and-register flow: native folder picker → the unchanged
/// `register_project` command. Used by the compact sidebar button and by the
/// ⌘K palette's "Add project" action so the flow lives in one place.
/// Returns the registered project, or `null` if the picker was cancelled.
export async function pickAndRegisterProject(): Promise<Project | null> {
  const folder = (await open({ directory: true, multiple: false })) as
    | string
    | null;
  if (!folder) return null; // picker cancelled
  return registerProject(folder);
}

export function AddProject({
  onAdded,
  compact = false,
}: {
  onAdded: (p: Project) => void;
  compact?: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function pick() {
    setError(null);
    setBusy(true);
    try {
      const p = await pickAndRegisterProject();
      if (p) onAdded(p);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  // Compact form: a header "+" affordance for the connections sidebar. The
  // registration error (not a git repo, already registered) surfaces in a small
  // popover under the button so the header layout stays fixed.
  if (compact) {
    return (
      <div className="add-project add-project--compact">
        <button
          className="add-project__icon"
          onClick={pick}
          disabled={busy}
          title="Add a git repo…"
          aria-label="Add project"
        >
          {busy ? "…" : "+"}
        </button>
        {error && <div className="add-project__error">{error}</div>}
      </div>
    );
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
