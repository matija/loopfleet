// Per-project sandbox write overrides: extra absolute paths spliced into each
// run's Seatbelt write grant (PRD M6 settings). Scoped to the selected project;
// reloads when the selection changes. The Rust setter rejects relative paths, so
// a bad override surfaces as an inline error rather than silently weakening the
// boundary.

import { useEffect, useState } from "react";
import { projectSandboxWrites, setProjectSandboxWrites } from "../commands";

export function SandboxOverrides({ projectId }: { projectId: string }) {
  const [text, setText] = useState("");
  // `loaded` gates the textarea until the persisted overrides arrive, so an
  // empty box doesn't read as "no overrides" while the load is still in flight.
  // `loadError` surfaces a load failure instead of the previous silent swallow
  // (which would have let the user save a blank list over a real one).
  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState<{ text: string; ok: boolean } | null>(null);

  useEffect(() => {
    setMsg(null);
    setLoaded(false);
    setLoadError(null);
    let cancelled = false;
    projectSandboxWrites(projectId)
      .then((paths) => {
        if (cancelled) return;
        setText(paths.join("\n"));
        setLoaded(true);
      })
      .catch((e) => {
        if (cancelled) return;
        setText("");
        setLoadError(String(e));
        setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [projectId]);

  async function save() {
    const paths = text
      .split("\n")
      .map((s) => s.trim())
      .filter(Boolean);
    setSaving(true);
    setMsg(null);
    try {
      await setProjectSandboxWrites(projectId, paths);
      setText(paths.join("\n"));
      setMsg({ text: "Saved", ok: true });
    } catch (e) {
      setMsg({ text: String(e), ok: false });
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="panel">
      <div className="panel__head">
        <h3>Sandbox write overrides</h3>
      </div>
      <p className="panel__hint">
        Extra absolute paths added to this project's per-run write grant. One per
        line. Never list the parent <code>.git</code> — commits stay app-owned.
      </p>
      {loadError ? (
        <p className="panel__error">
          Couldn’t load overrides: {loadError}. Saving will overwrite the
          existing list.
        </p>
      ) : !loaded ? (
        <p className="panel__loading">Loading overrides…</p>
      ) : null}
      <textarea
        className="overrides__ta"
        value={text}
        disabled={!loaded}
        onChange={(e) => setText(e.target.value)}
        placeholder="/absolute/path/per/line"
        spellCheck={false}
      />
      <div className="panel__actions">
        <button className="btn" onClick={save} disabled={saving || !loaded}>
          {saving ? "Saving…" : "Save overrides"}
        </button>
        {msg && (
          <span className={`msg ${msg.ok ? "msg--ok" : "msg--err"}`}>
            {msg.text}
          </span>
        )}
      </div>
    </section>
  );
}
