// The honest sandbox-boundary panel. The PRD names this "a trust feature, not a
// footnote": every run spawns under a macOS Seatbelt (`sandbox-exec`) profile,
// and the app must state plainly what that profile does and does NOT constrain.
// Overstating "sandboxed" is worse than stating the boundary narrowly, so the
// wording here mirrors the PRD Sandbox section verbatim in spirit: writes are
// confined to the worktree; reads and network are not.
//
// It opens like every other panel — `panel` / `panel__head` with the glyph its
// surface card carries, a one-line `panel__lead`, a closing `panel__foot` —
// rather than in the byte-identical `sandbox-panel__*` copies it used to own.
// A trust panel that looked hand-built next to Agents and Settings read as
// less a part of the app than the surfaces it is meant to vouch for.

import { BoxIcon } from "./Icon";

export function SandboxBoundaryPanel() {
  return (
    <section className="panel" aria-labelledby="sandbox-heading">
      <div className="panel__head">
        <BoxIcon size={16} className="icon panel__icon" />
        <h3 id="sandbox-heading">Sandbox boundary</h3>
        <span className="panel__tag">sandbox-exec · Seatbelt</span>
      </div>
      <p className="panel__lead">
        Agents run in full-auto with their own permission systems disabled. The
        OS sandbox profile rendered per run is the single security boundary — not
        a footnote.
      </p>

      <div className="sandbox-rules">
        <div className="sandbox-rule">
          <span className="sandbox-rule__badge sandbox-rule__badge--confined">
            Confined
          </span>
          <div className="sandbox-rule__text">
            <strong>Writes</strong>
            <p>
              Limited to the run's git worktree and its app-managed progress dir.
              The parent repo's <code>.git</code> is never writable — commits are
              app-owned.
            </p>
          </div>
        </div>

        <div className="sandbox-rule">
          <span className="sandbox-rule__badge sandbox-rule__badge--open">
            Open
          </span>
          <div className="sandbox-rule__text">
            <strong>Reads &amp; network</strong>
            <p>
              Not confined. A sandboxed agent can read anything your user account
              can — <code>~/.ssh</code>, <code>~/.aws</code>, other repos' env
              files — and POST it out. Only run plans you trust.
            </p>
          </div>
        </div>
      </div>

      <p className="panel__foot">
        Read-scope confinement and an egress allowlist are post-v1 hardening.
      </p>
    </section>
  );
}
