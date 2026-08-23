// Global app settings: default agent, default iteration count, concurrency
// cap, worktree retention.
// Loaded on mount, saved through the unchanged `get_settings`/`save_settings`
// commands. The launch control (a later M7 task) reads these defaults.

import { useEffect, useState } from "react";
import { getSettings, saveSettings, sweepWorktreesNow } from "../commands";
import type { Settings } from "../types";
import {
  DEFAULT_RETENTION_HOURS,
  retentionModeOf,
  retentionValue,
  type RetentionMode,
} from "../retention";

// The v1 agent keys (matches the adapters' discovery set). A small stable list;
// no need to derive it from `agent_status` here.
const AGENTS = ["claude", "pi", "cursor"];

// Human-readable byte count for the sweep toast (binary units, matches how
// most OS file browsers report disk usage).
function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

export function SettingsPanel() {
  const [settings, setSettings] = useState<Settings>({
    default_agent: "claude",
    default_iterations: 1,
    concurrency_cap: 3,
    worktree_retention_hours: DEFAULT_RETENTION_HOURS,
  });
  // `loaded` gates the form until the persisted settings arrive, so a user
  // can't edit the placeholder defaults and have their edits overwritten when
  // the load resolves. `loadError` surfaces a load failure instead of the
  // previous silent swallow (which left the user editing stale defaults).
  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  // Retention is edited as a mode plus a free-text hour count rather than
  // straight off `settings.worktree_retention_hours`: clearing the input
  // yields `Number("") === 0`, which would silently flip the mode to
  // "immediately" and unmount the input the user is still typing in. The two
  // are folded back into the single stored number on save.
  const [retentionMode, setRetentionMode] = useState<RetentionMode>("after");
  const [retentionHours, setRetentionHours] = useState(
    String(DEFAULT_RETENTION_HOURS),
  );
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState<{ text: string; ok: boolean } | null>(null);
  const [sweeping, setSweeping] = useState(false);
  const [sweepMsg, setSweepMsg] = useState<{ text: string; ok: boolean } | null>(
    null,
  );

  useEffect(() => {
    let cancelled = false;
    getSettings()
      .then((s) => {
        if (cancelled) return;
        setSettings(s);
        setRetentionMode(retentionModeOf(s.worktree_retention_hours));
        if (s.worktree_retention_hours > 0) {
          setRetentionHours(String(s.worktree_retention_hours));
        }
        setLoaded(true);
      })
      .catch((e) => {
        if (cancelled) return;
        setLoadError(String(e));
        setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function save() {
    const next: Settings = {
      default_agent: settings.default_agent,
      default_iterations: Math.max(1, settings.default_iterations || 1),
      concurrency_cap: Math.max(0, settings.concurrency_cap || 0),
      worktree_retention_hours: retentionValue(retentionMode, retentionHours),
    };
    setSaving(true);
    setMsg(null);
    try {
      await saveSettings(next);
      setSettings(next);
      // Reflect what actually got persisted, so a draft the fallback rejected
      // ("", "abc") doesn't keep showing next to the saved value.
      if (next.worktree_retention_hours > 0) {
        setRetentionHours(String(next.worktree_retention_hours));
      }
      setMsg({ text: "Saved", ok: true });
    } catch (e) {
      setMsg({ text: String(e), ok: false });
    } finally {
      setSaving(false);
    }
  }

  async function cleanUpNow() {
    setSweeping(true);
    setSweepMsg(null);
    try {
      const result = await sweepWorktreesNow();
      setSweepMsg({
        text:
          result.removed === 0
            ? "Nothing to clean up"
            : `Removed ${result.removed} worktree${result.removed === 1 ? "" : "s"}, reclaimed ${formatBytes(result.bytes_reclaimed)}`,
        ok: true,
      });
    } catch (e) {
      setSweepMsg({ text: String(e), ok: false });
    } finally {
      setSweeping(false);
    }
  }

  return (
    <section className="panel">
      <div className="panel__head">
        <h3>Settings</h3>
      </div>
      {loadError ? (
        <p className="panel__error">
          Couldn’t load settings: {loadError}. Showing defaults — saving will
          overwrite them.
        </p>
      ) : !loaded ? (
        <p className="panel__loading">Loading settings…</p>
      ) : null}
      <div className="form-grid">
        <label className="field">
          <span>Default agent</span>
          <select
            value={settings.default_agent}
            disabled={!loaded}
            onChange={(e) =>
              setSettings({ ...settings, default_agent: e.target.value })
            }
          >
            {AGENTS.map((a) => (
              <option key={a} value={a}>
                {a}
              </option>
            ))}
          </select>
        </label>
        <label className="field">
          <span>Default iterations</span>
          <input
            type="number"
            min={1}
            max={50}
            value={settings.default_iterations}
            disabled={!loaded}
            onChange={(e) =>
              setSettings({
                ...settings,
                default_iterations: Number(e.target.value),
              })
            }
          />
        </label>
        <label className="field">
          <span>
            Concurrency cap <em>(0 = unlimited)</em>
          </span>
          <input
            type="number"
            min={0}
            max={20}
            value={settings.concurrency_cap}
            disabled={!loaded}
            onChange={(e) =>
              setSettings({
                ...settings,
                concurrency_cap: Number(e.target.value),
              })
            }
          />
        </label>
        <label className="field">
          <span>Worktree retention</span>
          <select
            value={retentionMode}
            disabled={!loaded}
            onChange={(e) => setRetentionMode(e.target.value as RetentionMode)}
          >
            <option value="immediately">Immediately</option>
            <option value="after">After a number of hours</option>
            <option value="never">Never</option>
          </select>
          {retentionMode === "after" && (
            <input
              type="number"
              min={1}
              aria-label="Retention hours"
              value={retentionHours}
              disabled={!loaded}
              onChange={(e) => setRetentionHours(e.target.value)}
            />
          )}
          {/* All three modes spelled out, not just the selected one, so the
            * trade-off (disk vs. being able to revisit a finished run) is
            * legible without cycling the dropdown. */}
          <span className="field__hint">
            When a finished run’s worktree is deleted, measured from when it
            finished. <em>Immediately</em> reclaims disk as soon as a run ends;{" "}
            <em>after a number of hours</em> keeps it that long so you can still
            open the diff; <em>never</em> keeps it indefinitely. Accepted runs
            are always swept — their diff has already landed.
          </span>
        </label>
      </div>
      <div className="panel__actions">
        <button className="btn" onClick={save} disabled={saving || !loaded}>
          {saving ? "Saving…" : "Save settings"}
        </button>
        {msg && (
          <span className={`msg ${msg.ok ? "msg--ok" : "msg--err"}`}>
            {msg.text}
          </span>
        )}
      </div>
      <div className="panel__actions">
        <button
          className="btn"
          onClick={cleanUpNow}
          disabled={sweeping || !loaded}
        >
          {sweeping ? "Cleaning up…" : "Clean up now"}
        </button>
        {sweepMsg && (
          <span className={`msg ${sweepMsg.ok ? "msg--ok" : "msg--err"}`}>
            {sweepMsg.text}
          </span>
        )}
      </div>
    </section>
  );
}
