// Global app settings: default agent, default iteration count, concurrency
// cap, worktree retention.
//
// Laid out as labelled sections (run defaults / worktrees / appearance), each
// led by a glyph so a group is findable by shape before its title is read.
// Within a section, controls are sized to what they hold — an hour count is a
// few digits wide, an agent name a word — and related fields share a row, so
// the panel reads as a handful of decisions rather than one long column.
// Loaded on mount, saved through the unchanged `get_settings`/`save_settings`
// commands. The launch control (a later M7 task) reads these defaults.
//
// The two settings that need more than a label to explain them — worktree
// retention and theme — carry a `Hint` rather than a paragraph, so their
// explanations sit behind an info affordance instead of pushing the controls
// under them down the panel.
//
// Theme is the one field here that is *not* backend state: it's a per-device
// display preference App owns and persists to localStorage, so it applies on
// pick rather than on Save. It lives in this panel anyway because this is
// where a user looks for app-wide preferences.

import { useEffect, useState } from "react";
import { getSettings, saveSettings, sweepWorktreesNow } from "../commands";
import type { Settings } from "../types";
import {
  DEFAULT_RETENTION_HOURS,
  retentionModeOf,
  retentionValue,
  type RetentionMode,
} from "../retention";
import { isThemeId, THEMES, type ThemeId } from "../themes";
import { Select } from "./Select";
import { Hint } from "./Hint";
import { NumberField } from "./NumberField";
import { AgentIcon, DotIcon, FolderIcon } from "./Icon";

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

export function SettingsPanel({
  themeId,
  onThemeChange,
}: {
  themeId: ThemeId;
  onThemeChange: (id: ThemeId) => void;
}) {
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
      <section className="settings-section">
        <div className="settings-section__head">
          <AgentIcon size={16} className="icon settings-section__icon" />
          <h4 className="settings-section__title">Run defaults</h4>
        </div>
        <div className="settings-row">
          <label className="field">
            <span>Default agent</span>
            <Select
              className="control--name"
              value={settings.default_agent}
              disabled={!loaded}
              onChange={(v) => setSettings({ ...settings, default_agent: v })}
              options={AGENTS.map((a) => ({ value: a, label: a }))}
            />
          </label>
        </div>
        {/* Iterations and concurrency answer one question together — how much
          * work a launch does at once — so they sit on a single row. */}
        <div className="settings-row">
          <label className="field">
            <span>Default iterations</span>
            <NumberField
              className="control--count"
              min={1}
              max={50}
              value={settings.default_iterations}
              disabled={!loaded}
              onChange={(v) =>
                setSettings({
                  ...settings,
                  default_iterations: v,
                })
              }
            />
          </label>
          <label className="field">
            <span>
              Concurrency cap <em>(0 = unlimited)</em>
            </span>
            <NumberField
              className="control--count"
              min={0}
              max={20}
              value={settings.concurrency_cap}
              disabled={!loaded}
              onChange={(v) =>
                setSettings({
                  ...settings,
                  concurrency_cap: v,
                })
              }
            />
          </label>
        </div>
      </section>

      <section className="settings-section">
        <div className="settings-section__head">
          <FolderIcon size={16} className="icon settings-section__icon" />
          <h4 className="settings-section__title">Worktrees</h4>
        </div>
        {/* The hour count is an argument to the "after a delay" mode, not a
          * setting of its own, so it sits beside the mode it qualifies. */}
        <div className="settings-row">
          <label className="field">
            <span>Delete finished worktrees</span>
            <Select
              className="control--mode"
              value={retentionMode}
              disabled={!loaded}
              onChange={(v) => setRetentionMode(v as RetentionMode)}
              options={[
                { value: "immediately", label: "Immediately" },
                { value: "after", label: "After a delay" },
                { value: "never", label: "Never" },
              ]}
            />
          </label>
          {retentionMode === "after" && (
            <label className="field">
              <span>Hours</span>
              <NumberField
                className="control--count"
                min={1}
                aria-label="Retention hours"
                value={Number(retentionHours) || 0}
                disabled={!loaded}
                onChange={(v) => setRetentionHours(String(v))}
              />
            </label>
          )}
        </div>
        {/* All three modes spelled out, not just the selected one, so the
          * trade-off (disk vs. being able to revisit a finished run) is
          * legible without cycling the dropdown. That's four sentences, which
          * is why it's disclosed rather than printed inline: at full length it
          * would push "Clean up now" and the Appearance section down the
          * panel. */}
        <Hint
          summary="Measured from when a run finished."
          label="worktree deletion"
        >
          <p>
            <em>Immediately</em> reclaims disk as soon as a run ends;{" "}
            <em>after a delay</em> keeps the worktree that many hours so you can
            still open the diff; <em>never</em> keeps it indefinitely.
          </p>
          <p>
            Accepted runs are always swept — their diff has already landed.
          </p>
        </Hint>
        <div className="panel__actions">
          <button
            className="btn btn--secondary"
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

      <section className="settings-section">
        <div className="settings-section__head">
          {/* A filled dot reads as a color swatch — the closest glyph in the
            * family to "how this looks". */}
          <DotIcon size={16} className="icon settings-section__icon" />
          <h4 className="settings-section__title">Appearance</h4>
        </div>
        <div className="settings-row">
          <label className="field">
            <span>Theme</span>
            <Select
              className="control--name"
              value={themeId}
              onChange={(v) => {
                // The option values are exactly THEMES ids; the guard is only
                // to keep the cast honest.
                if (isThemeId(v)) onThemeChange(v);
              }}
              options={THEMES.map((theme) => ({ value: theme.id, label: theme.label }))}
            />
          </label>
        </div>
        {/* The summary carries the part a user acts on — there's no Save to
          * press for this one — and the reason why sits behind the affordance. */}
        <Hint
          summary="Applies immediately, on this device only."
          label="the theme setting"
        >
          <p>
            Theme is a per-device display preference rather than part of the
            settings above, so it takes effect the moment you pick it and
            “Save settings” doesn’t affect it.
          </p>
        </Hint>
      </section>

      <div className="panel__actions">
        <button className="btn btn--primary" onClick={save} disabled={saving || !loaded}>
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
