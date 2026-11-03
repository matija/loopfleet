// Per-project sandbox write overrides: extra absolute paths spliced into each
// run's Seatbelt write grant (PRD M6 settings). Scoped to the selected project;
// reloads when the selection changes. The Rust setter rejects relative paths, so
// a bad override surfaces as an inline error rather than silently weakening the
// boundary.
//
// It opens closed. Nothing here is needed to run a task — the default write
// grant already covers the worktree — so a permanently expanded textarea at
// the foot of every plan reads as a step in launching a run rather than the
// rarely-touched escape hatch it is. The head is a disclosure instead, with
// the current override count beside it, so a project that *has* overrides
// still says so without the body being on screen; the state is persisted per
// disclosure (see sidebarCollapse.ts), so someone who does keep it open
// keeps it open across reloads and project switches.
//
import { useEffect, useState } from "react";
import {
  projectSandboxWrites,
  setProjectSandboxWrites,
} from "../commands";
import { useSidebarCollapsed } from "../sidebarCollapse";
import { Button } from "./Button";
import { BoxIcon, ChevronRightIcon } from "./Icon";

/// The count shown beside the collapsed head, so the panel reports whether
/// this project has overrides without being opened. Blank until the load
/// lands — "none" before then would be a claim the app can't make yet.
export function overrideSummary(paths: string[], loaded: boolean): string {
  if (!loaded) return "";
  const count = paths.length === 1 ? "1 path" : `${paths.length} paths`;
  return paths.length ? count : "none";
}

export function SandboxOverrides({ projectId }: { projectId: string }) {
  // One id for the panel, not one per project: the choice being remembered is
  // "do I work with sandbox overrides", which isn't a per-repo preference.
  const [collapsed, toggleCollapsed] = useSidebarCollapsed(
    "panel:sandbox-writes",
    true,
  );
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

  // Counted off the saved-shaped list, not the raw textarea, so blank lines
  // and stray whitespace don't inflate the badge.
  const paths = text
    .split("\n")
    .map((s) => s.trim())
    .filter(Boolean);
  const summary = overrideSummary(paths, loaded && !loadError);

  return (
    <section className={`panel sandbox-overrides${collapsed ? " panel--collapsed" : ""}`}>
      <div className="panel__head">
        <h3 className="panel__head-title">
          <button
            type="button"
            className="panel__disclosure"
            aria-expanded={!collapsed}
            onClick={toggleCollapsed}
          >
            <ChevronRightIcon size={12} className="disclosure__chevron" />
            <BoxIcon size={16} className="icon panel__icon" />
            Sandbox permissions
          </button>
        </h3>
        {summary && <span className="panel__count">{summary}</span>}
      </div>
      {!collapsed && (
        <>
          {loadError ? (
            <p className="panel__error">
              Couldn’t load permissions: {loadError}. Reload this project before saving.
            </p>
          ) : !loaded ? (
            <p className="panel__loading">Loading overrides…</p>
          ) : null}
          <textarea
            className="overrides__ta"
            aria-label="Extra writable paths, one absolute path per line"
            value={text}
            disabled={!loaded || !!loadError || saving}
            onChange={(e) => setText(e.target.value)}
            placeholder="/absolute/path/per/line"
            spellCheck={false}
          />
          <div className="panel__actions">
            <Button variant="primary" onClick={save} disabled={saving || !loaded || !!loadError}>
              {saving ? "Saving…" : "Save permissions"}
            </Button>
            {msg && (
              <span className={`msg ${msg.ok ? "msg--ok" : "msg--err"}`}>
                {msg.text}
              </span>
            )}
          </div>
        </>
      )}
    </section>
  );
}
