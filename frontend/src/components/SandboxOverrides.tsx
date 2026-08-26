// Per-project sandbox write overrides: extra absolute paths spliced into each
// run's Seatbelt write grant (PRD M6 settings). Scoped to the selected project;
// reloads when the selection changes. The Rust setter rejects relative paths, so
// a bad override surfaces as an inline error rather than silently weakening the
// boundary.
//
// Presented in the shared panel vocabulary: a glyph-led `panel__head`, one
// `panel__lead` line, and a single `panel__actions` row whose Save is a
// `Button` with an explicit variant rather than a hand-written `.btn` class.
// The reason a path list needs care — that the parent `.git` must never be on
// it — is three sentences long, so it rides a `Hint` beside the field like the
// settings that need more than a label do, instead of a paragraph that pushes
// the textarea down the panel.

import { useEffect, useState } from "react";
import { projectSandboxWrites, setProjectSandboxWrites } from "../commands";
import { Button } from "./Button";
import { Hint } from "./Hint";
import { BoxIcon } from "./Icon";

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
        <BoxIcon size={16} className="icon panel__icon" />
        <h3>Sandbox write overrides</h3>
      </div>
      <p className="panel__lead">
        Extra absolute paths added to this project's per-run write grant, one per
        line.
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
      {/* The summary carries the rule; the reason it matters — which is the
        * whole sandbox argument in miniature — sits behind the affordance. */}
      <Hint
        summary="Never list the parent repo’s .git."
        label="sandbox write overrides"
      >
        <p>
          Commits are app-owned: loopfleet writes them through the parent repo
          itself, so no run ever needs write access to <em>.git</em>. Granting it
          would let an agent rewrite history outside its own worktree — the one
          thing this boundary exists to prevent.
        </p>
        <p>
          Paths must be absolute; a relative one is rejected on save rather than
          quietly dropped, so a typo can’t widen the grant by accident.
        </p>
      </Hint>
      <div className="panel__actions">
        <Button variant="primary" onClick={save} disabled={saving || !loaded}>
          {saving ? "Saving…" : "Save overrides"}
        </Button>
        {msg && (
          <span className={`msg ${msg.ok ? "msg--ok" : "msg--err"}`}>
            {msg.text}
          </span>
        )}
      </div>
    </section>
  );
}
