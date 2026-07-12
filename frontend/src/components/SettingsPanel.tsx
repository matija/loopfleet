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
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState<{ text: string; ok: boolean } | null>(null);

  useEffect(() => {
    // Fall back to the defaults already in state if the load fails.
    getSettings()
      .then(setSettings)
      .catch(() => {});
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
      <div className="form-grid">
        <label className="field">
          <span>Default agent</span>
          <select
            value={settings.default_agent}
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
        <button className="btn" onClick={save} disabled={saving}>
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
