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
// Each section's head carries a quiet Reset that puts that section's defaults
// (settingsDefaults.ts, mirroring `store::Settings::default`) back into the
// form without saving — the user reads the restored values next to everything
// else and presses Save to commit them, so a mis-aimed Reset costs nothing.
// It's per section rather than one panel-wide button because sections are what
// a user thinks in here, and it's disabled while the section already holds its
// defaults, so it never claims there's something to undo when there isn't.
//
// The panel's commands share one footer row instead of a button per section:
// Save (primary) and Clean up now (quiet). Clean up now used to sit under
// Worktrees in a row of its own, which read as a second call to action of
// equal weight — the retention hint above it already says what it does, and
// pressing it is a chore rather than the point of the panel.
//
// Theme is the one field here that is *not* backend state: it's a per-device
// display preference App owns and persists to localStorage, so it applies on
// pick rather than on Save. It lives in this panel anyway because this is
// where a user looks for app-wide preferences.
//
// Because there's no Save to undo a theme, the picker is paired with a live
// miniature of the app (ThemePreview.tsx) that paints whichever option the
// dropdown is highlighting — so "what does Rosé Pine Moon look like?" is
// answered by looking rather than by picking it and picking back. With the
// popup closed the miniature shows the applied theme, which makes it a
// standing sample of the current one rather than a box that goes blank.

import { useEffect, useState } from "react";
import { appSettings, saveAppSettings } from "../appData";
import { sweepWorktreesNow } from "../commands";
import type { Settings } from "../types";
import {
  DEFAULT_RETENTION_HOURS,
  retentionModeOf,
  retentionValue,
  type RetentionMode,
} from "../retention";
import {
  DEFAULT_AUTOPILOT,
  DEFAULT_RUN_DEFAULTS,
  DEFAULT_WORKTREES,
  isAutopilotAtDefault,
  isRunDefaultsAtDefault,
  isThemeAtDefault,
  isWorktreesAtDefault,
} from "../settingsDefaults";
import {
  DEFAULT_THEME_ID,
  isThemeId,
  previewThemeId,
  THEMES,
  type ThemeId,
} from "../themes";
import { Button } from "./Button";
import { Select } from "./Select";
import { Hint } from "./Hint";
import { NumberField } from "./NumberField";
import { ThemePreview } from "./ThemePreview";
import { AgentIcon, DotIcon, FolderIcon, PlayIcon, SettingsIcon } from "./Icon";

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

// The per-section Reset. Text-weight (`quiet`) so it reads as an escape hatch
// beside the section title rather than competing with Save; the accessible
// name says which section it resets, since three buttons labelled "Reset"
// would otherwise be indistinguishable to a screen reader.
function SectionReset({
  section,
  disabled,
  onReset,
}: {
  section: string;
  disabled: boolean;
  onReset: () => void;
}) {
  return (
    <Button
      variant="quiet"
      className="panel-section__reset"
      onClick={onReset}
      disabled={disabled}
      aria-label={`Reset ${section} to defaults`}
      // Section-neutral wording: the accessible name already says which
      // section this is, and "Appearance"/"Run defaults" don't share a verb
      // form that would read well in one sentence.
      title={
        disabled
          ? "This section is already at its defaults"
          : "Restore this section's defaults"
      }
    >
      Reset
    </Button>
  );
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
    default_model: null,
    default_iterations: 1,
    concurrency_cap: 3,
    worktree_retention_hours: DEFAULT_RETENTION_HOURS,
    cleanup_after_merge: true,
    auto_merge_enabled: DEFAULT_AUTOPILOT.auto_merge_enabled,
    auto_merge_countdown_seconds: DEFAULT_AUTOPILOT.auto_merge_countdown_seconds,
    auto_advance_enabled: DEFAULT_AUTOPILOT.auto_advance_enabled,
    auto_advance_delay_seconds: DEFAULT_AUTOPILOT.auto_advance_delay_seconds,
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
  const [cleanupAfterMerge, setCleanupAfterMerge] = useState(true);
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState<{ text: string; ok: boolean } | null>(null);
  // A reset changes the form and nothing else, so the panel says so once —
  // otherwise the restored values look indistinguishable from saved ones and
  // a user could close the panel believing the reset had taken.
  const [resetNote, setResetNote] = useState<string | null>(null);
  const [sweeping, setSweeping] = useState(false);
  const [sweepMsg, setSweepMsg] = useState<{ text: string; ok: boolean } | null>(
    null,
  );
  // The theme option the picker is currently highlighting, or null when its
  // popup is closed. Only the preview reads it — highlighting is not picking,
  // so nothing is applied or persisted until the option is committed.
  const [highlightedTheme, setHighlightedTheme] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    appSettings()
      .then((s) => {
        if (cancelled) return;
        setSettings(s);
        setRetentionMode(retentionModeOf(s.worktree_retention_hours));
        if (s.worktree_retention_hours > 0) {
          setRetentionHours(String(s.worktree_retention_hours));
        }
        setCleanupAfterMerge(s.cleanup_after_merge);
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
      default_model: settings.default_model,
      default_iterations: Math.max(1, settings.default_iterations || 1),
      concurrency_cap: Math.max(0, settings.concurrency_cap || 0),
      worktree_retention_hours: retentionValue(retentionMode, retentionHours),
      cleanup_after_merge: cleanupAfterMerge,
      auto_merge_enabled: settings.auto_merge_enabled,
      auto_merge_countdown_seconds: Math.max(
        1,
        settings.auto_merge_countdown_seconds || 1,
      ),
      auto_advance_enabled: settings.auto_advance_enabled,
      auto_advance_delay_seconds: Math.max(
        1,
        settings.auto_advance_delay_seconds || 1,
      ),
    };
    setSaving(true);
    setMsg(null);
    setResetNote(null);
    try {
      await saveAppSettings(next);
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

  // Each reset writes defaults into the draft state only. `msg` is cleared
  // alongside: a "Saved" from a moment ago would otherwise sit next to values
  // that haven't been saved.
  function resetRunDefaults() {
    setSettings({ ...settings, ...DEFAULT_RUN_DEFAULTS });
    setMsg(null);
    setResetNote("Run defaults restored — review, then save.");
  }

  function resetWorktrees() {
    setRetentionMode(DEFAULT_WORKTREES.mode);
    setRetentionHours(DEFAULT_WORKTREES.hours);
    setCleanupAfterMerge(DEFAULT_WORKTREES.cleanupAfterMerge);
    setMsg(null);
    setResetNote("Worktree defaults restored — review, then save.");
  }

  function resetAutopilot() {
    setSettings({ ...settings, ...DEFAULT_AUTOPILOT });
    setMsg(null);
    setResetNote("Autopilot defaults restored — review, then save.");
  }

  // Appearance is the exception: theme isn't part of what Save writes, so its
  // reset applies at once, exactly like picking the default from the list.
  // No note either — there's nothing left for the user to commit.
  function resetAppearance() {
    onThemeChange(DEFAULT_THEME_ID);
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
        <SettingsIcon size={16} className="icon panel__icon" />
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
      <section className="panel-section">
        <div className="panel-section__head">
          <AgentIcon size={16} className="icon panel-section__icon" />
          <h4 className="panel-section__title">Run defaults</h4>
          <SectionReset
            section="Run defaults"
            disabled={!loaded || isRunDefaultsAtDefault(settings)}
            onReset={resetRunDefaults}
          />
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

      <section className="panel-section">
        <div className="panel-section__head">
          <FolderIcon size={16} className="icon panel-section__icon" />
          <h4 className="panel-section__title">Worktrees</h4>
          <SectionReset
            section="Worktree settings"
            disabled={
              !loaded ||
              isWorktreesAtDefault({
                mode: retentionMode,
                hours: retentionHours,
                cleanupAfterMerge,
              })
            }
            onReset={resetWorktrees}
          />
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
          * would push the Appearance section and the action row down the
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
        <div className="settings-row">
          <label className="field field--checkbox">
            <input
              type="checkbox"
              checked={cleanupAfterMerge}
              disabled={!loaded}
              onChange={(e) => setCleanupAfterMerge(e.target.checked)}
            />
            <span>Delete worktree and branch after merge</span>
          </label>
        </div>
        <p className="field-note">
          When a run is accepted, its worktree and <code>agent/…</code> branch
          are deleted. The run's history, diffs and reports are kept either
          way.
        </p>
      </section>

      <section className="panel-section">
        <div className="panel-section__head">
          <PlayIcon size={16} className="icon panel-section__icon" />
          <h4 className="panel-section__title">Autopilot</h4>
          <SectionReset
            section="Autopilot"
            disabled={!loaded || isAutopilotAtDefault(settings)}
            onReset={resetAutopilot}
          />
        </div>
        <div className="settings-row">
          <label className="field field--checkbox">
            <input
              type="checkbox"
              checked={settings.auto_advance_enabled}
              disabled={!loaded}
              onChange={(e) =>
                setSettings({
                  ...settings,
                  auto_advance_enabled: e.target.checked,
                })
              }
            />
            <span>Auto-advance to the next plan step</span>
          </label>
        </div>
        <p className="field-note">
          Default: on. When a run finishes ready to continue, the plan's next
          task launches on its own after a countdown, instead of waiting for
          you to start it.
        </p>
        <div className="settings-row">
          <label className="field">
            <span>Auto-advance delay (seconds)</span>
            <NumberField
              className="control--count"
              min={1}
              max={3600}
              value={settings.auto_advance_delay_seconds}
              disabled={!loaded || !settings.auto_advance_enabled}
              onChange={(v) =>
                setSettings({
                  ...settings,
                  auto_advance_delay_seconds: v,
                })
              }
            />
          </label>
        </div>
        <p className="field-note">
          Default: 5 seconds. How long the countdown waits before advancing,
          giving you a window to cancel it.
        </p>
        <div className="settings-row">
          <label className="field field--checkbox">
            <input
              type="checkbox"
              checked={settings.auto_merge_enabled}
              disabled={!loaded}
              onChange={(e) =>
                setSettings({
                  ...settings,
                  auto_merge_enabled: e.target.checked,
                })
              }
            />
            <span>Auto-merge accepted runs</span>
          </label>
        </div>
        <p className="field-note">
          Default: on. Once you accept a run, its branch merges on its own
          after a countdown, instead of waiting for you to merge it.
        </p>
        <div className="settings-row">
          <label className="field">
            <span>Auto-merge countdown (seconds)</span>
            <NumberField
              className="control--count"
              min={1}
              max={3600}
              value={settings.auto_merge_countdown_seconds}
              disabled={!loaded || !settings.auto_merge_enabled}
              onChange={(v) =>
                setSettings({
                  ...settings,
                  auto_merge_countdown_seconds: v,
                })
              }
            />
          </label>
        </div>
        <p className="field-note">
          Default: 10 seconds. How long the countdown waits before merging,
          giving you a window to cancel it.
        </p>
      </section>

      <section className="panel-section">
        <div className="panel-section__head">
          {/* A filled dot reads as a color swatch — the closest glyph in the
            * family to "how this looks". */}
          <DotIcon size={16} className="icon panel-section__icon" />
          <h4 className="panel-section__title">Appearance</h4>
          {/* Not gated on `loaded`: theme comes from localStorage, not the
            * settings load this panel waits on. */}
          <SectionReset
            section="Appearance"
            disabled={isThemeAtDefault(themeId)}
            onReset={resetAppearance}
          />
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
              onHighlight={(v) => setHighlightedTheme(v ?? null)}
              options={THEMES.map((theme) => ({ value: theme.id, label: theme.label }))}
            />
          </label>
          {/* Beside the picker rather than under it: the two are one decision,
            * and the row already wraps on a narrow panel. */}
          <ThemePreview themeId={previewThemeId(highlightedTheme, themeId)} />
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
          <p>
            The sample beside the list previews whichever theme you’re
            hovering or arrowing over, so you can judge one without switching
            to it.
          </p>
        </Hint>
      </section>

      {/* One action row for the panel’s two commands, ranked rather than
        * stacked: Save is the reason the panel is open, so it takes the filled
        * primary; Clean up now is a maintenance chore you may never press, so
        * it rides beside Save at text weight. Each keeps its own message,
        * printed after the pair so neither result is mistaken for the other. */}
      <div className="panel__actions">
        <Button variant="primary" onClick={save} disabled={saving || !loaded}>
          {saving ? "Saving…" : "Save settings"}
        </Button>
        <Button
          variant="quiet"
          onClick={cleanUpNow}
          disabled={sweeping || !loaded}
          title="Delete finished worktrees that are past their retention window"
        >
          {sweeping ? "Cleaning up…" : "Clean up now"}
        </Button>
        {msg ? (
          <span className={`msg ${msg.ok ? "msg--ok" : "msg--err"}`}>
            {msg.text}
          </span>
        ) : resetNote ? (
          <span className="msg">{resetNote}</span>
        ) : null}
        {sweepMsg && (
          <span className={`msg ${sweepMsg.ok ? "msg--ok" : "msg--err"}`}>
            {sweepMsg.text}
          </span>
        )}
      </div>
    </section>
  );
}
