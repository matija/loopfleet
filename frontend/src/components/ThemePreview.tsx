// A miniature of the app rendered in an arbitrary theme, so the settings
// picker can answer "what would this look like?" before the pick is made.
//
// It works because tokens.css scopes its color blocks to [data-theme="<id>"]
// on *any* element rather than the document root: putting the attribute on
// this one box makes every --c-* token inside it resolve to that theme's
// palette, while the rest of the panel stays in the applied one. Nothing here
// touches document.documentElement, so browsing the dropdown never repaints
// the app.
//
// The sample is built from the app's real classes wherever one exists —
// `.status-pill` from status.css, the dock's `.run-chip__*` row parts — rather
// than lookalikes drawn for the preview. That's the whole point of a preview:
// if a run row's color moves, the miniature moves with it instead of quietly
// becoming a picture of an older app. Only the frame and the card are
// preview-owned (theme-preview.css), because the real ones are sized for a
// full panel.
//
// Four surfaces, chosen to span the palette a theme actually has to get
// right: a raised card (surface + border + shadow), a status pill (the
// semantic hues), a pair of run rows including the selected step (state
// fills), and monospace text (the diff/log face, where a theme's contrast
// shows up hardest). The content is fixed, invented, and decorative — the
// whole box is one `role="img"` labelled with the theme name, so a screen
// reader gets "Preview of the Dark theme" rather than a fake run to explore.

import { CheckIcon, PlayIcon } from "./Icon";
import { RUN_STATUS_ICON, RUN_STATUS_LABEL } from "../status";
import { themeById, type ThemeId } from "../themes";

const PILL_STATUS = "running" as const;

export function ThemePreview({ themeId }: { themeId: ThemeId }) {
  const PillIcon = RUN_STATUS_ICON[PILL_STATUS];

  return (
    <div
      className="theme-preview"
      data-theme={themeId}
      role="img"
      aria-label={`Preview of the ${themeById(themeId).label} theme`}
    >
      <div className="theme-preview__card">
        <div className="theme-preview__head">
          <span className="theme-preview__title">Add retry budget</span>
          <span className={`status-pill status-pill--${PILL_STATUS}`}>
            <PillIcon size={12} className="status-pill__icon" />
            {RUN_STATUS_LABEL[PILL_STATUS]}
          </span>
        </div>

        <div className="theme-preview__row theme-preview__row--selected">
          <span className="run-chip__status run-chip__status--running">
            <PlayIcon size={12} />
          </span>
          <span className="run-chip__task">iteration 2 of 3</span>
          <span className="run-chip__meta run-elapsed">1m 04s</span>
        </div>
        <div className="theme-preview__row">
          <span className="run-chip__status run-chip__status--completed">
            <CheckIcon size={12} />
          </span>
          <span className="run-chip__task">iteration 1 of 3</span>
          <span className="run-chip__meta run-elapsed">2m 41s</span>
        </div>

        <code className="theme-preview__mono">retry.rs +24 −3</code>
      </div>
    </div>
  );
}
