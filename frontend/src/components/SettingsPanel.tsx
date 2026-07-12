// Global app settings: default agent, default iteration count, concurrency cap.
// Loaded on mount, saved through the unchanged `get_settings`/`save_settings`
// commands. The launch control (a later M7 task) reads these defaults.

import { useEffect, useState } from "react";
import { getSettings, saveSettings } from "../commands";
import type { Settings } from "../types";

// The v1 agent keys (matches the adapters' discovery set). A small stable list;
// no need to derive it from `agent_status` here.
const AGENTS = ["claude", "pi", "cursor"];

export function SettingsPanel() {
  const [settings, setSettings] = useState<Settings>({
    default_agent: "claude",
    default_iterations: 5,
    concurrency_cap: 3,
  });
  // `loaded` gates the form until the persisted settings arrive, so a user
  // can't edit the placeholder defaults and have their edits overwritten when
  // the load resolves. `loadError` surfaces a load failure instead of the
  // previous silent swallow (which left the user editing stale defaults).
  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState<{ text: string; ok: boolean } | null>(null);

  useEffect(() => {
    let cancelled = false;
    getSettings()
      .then((s) => {
        if (cancelled) return;
        setSettings(s);
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
    };
    setSaving(true);
    setMsg(null);
    try {
      await saveSettings(next);
      setSettings(next);
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
    </section>
  );
}
