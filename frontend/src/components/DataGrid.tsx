// Typed event grid (Workbench task 6): the database-client "typed data grid"
// analog. Renders normalized events as rows with columns (#, seq, type, detail,
// ts); the `type` column shows each `NormalizedEvent` variant as a colored enum
// pill (stable color per variant), and an empty detail/ts renders as a muted
// `NULL`-style pill. Reused by both the live run view and the run timeline so
// the streamed and replayed logs share one vocabulary and one layout.

import type { NormalizedEvent } from "../types";

/// One grid row: the event's log position, its recorded/observed time (unix
/// millis, or `null` when unknown — the live stream carries no persisted ts),
/// and the normalized event itself.
export type GridRow = { seq: number; ts: number | null; event: NormalizedEvent };

type Pill = { label: string; tone: string };

// The enum vocabulary: one PascalCase label + a stable tone per variant. Tones
// map to `.grid-pill--<tone>` colors in grid.css; related variants (the turn
// lifecycle) share a tone while keeping distinct labels.
export function eventPill(e: NormalizedEvent): Pill {
  switch (e.kind) {
    case "turn_started":
      return { label: "Start", tone: "neutral" };
    case "assistant_text":
      return { label: "Agent", tone: "text" };
    case "reasoning":
      return { label: "Thinking", tone: "neutral" };
    case "tool_call":
      return { label: "Tool", tone: "neutral" };
    case "tool_result":
      return { label: "Result", tone: e.ok ? "neutral" : "error" };
    case "command_run":
      return { label: "Command", tone: "neutral" };
    case "turn_completed":
      return { label: "Complete", tone: "neutral" };
    case "needs_approval":
      return { label: "Approval", tone: "warn" };
    case "failed":
      return { label: "Error", tone: "error" };
    case "ended":
      return { label: "End", tone: "neutral" };
    case "file_changed":
      return { label: "File", tone: "neutral" };
    default: {
      const exhaustive: never = e;
      return exhaustive;
    }
  }
}

// The `detail` cell content: whatever payload the variant carries, or "" for the
// payload-less events (rendered as the NULL pill).
export function eventDetail(e: NormalizedEvent): string {
  switch (e.kind) {
    case "assistant_text":
    case "reasoning":
      return e.text;
    case "tool_call":
      return `${e.name} · ${e.input_excerpt}`;
    case "tool_result":
      return e.output_excerpt;
    case "command_run":
      return e.exit === null ? e.cmd : `${e.cmd}  → exit ${e.exit}`;
    case "turn_completed":
      return `${e.usage.input_tokens} in · ${e.usage.output_tokens} out tokens`;
    case "failed":
      return e.reason;
    case "file_changed":
      return e.path;
    case "turn_started":
    case "needs_approval":
    case "ended":
      return "";
    default: {
      const exhaustive: never = e;
      return exhaustive;
    }
  }
}

/// The searchable text for the command bar's `WHERE …` filter: the pill label
/// plus the detail, so a query matches on either the event type or its content.
export function eventText(e: NormalizedEvent): string {
  return `${eventPill(e).label} ${eventDetail(e)}`;
}

function formatTs(ts: number | null): string | null {
  if (ts === null) return null;
  const d = new Date(ts);
  return d.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function DataGrid({ rows }: { rows: GridRow[] }) {
  return (
    <div className="data-grid" role="table" aria-label="Events">
      <div className="data-grid__row data-grid__row--head" role="row">
        <span className="data-grid__cell data-grid__cell--type">type</span>
        <span className="data-grid__cell data-grid__cell--detail">detail</span>
        <span className="data-grid__cell data-grid__cell--ts">ts</span>
      </div>
      {rows.map((r) => {
        const pill = eventPill(r.event);
        const detail = eventDetail(r.event);
        const ts = formatTs(r.ts);
        return (
          <div className="data-grid__row" role="row" key={r.seq}>
            <span className="data-grid__cell data-grid__cell--type">
              <span className={`grid-pill grid-pill--${pill.tone}`}>
                {pill.label}
              </span>
            </span>
            <span
              className="data-grid__cell data-grid__cell--detail"
              title={detail || undefined}
            >
              {detail ? detail : <span className="grid-null">NULL</span>}
            </span>
            <span className="data-grid__cell data-grid__cell--ts">
              {ts ?? <span className="grid-null">NULL</span>}
            </span>
          </div>
        );
      })}
    </div>
  );
}

/// Format a millisecond span the way a database-client footer times a query:
/// `42ms`, `1.3s`, `2m 05s`, `1h 04m`. `null`/negative (no usable span) → `—`.
export function formatDuration(ms: number | null): string {
  if (ms === null || ms < 0 || !Number.isFinite(ms)) return "—";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)}s`;
  const totalSecs = Math.round(s);
  const m = Math.floor(totalSecs / 60);
  const rs = totalSecs % 60;
  if (m < 60) return `${m}m ${rs.toString().padStart(2, "0")}s`;
  const h = Math.floor(m / 60);
  const rm = m % 60;
  return `${h}h ${rm.toString().padStart(2, "0")}m`;
}

/// The wall-clock span covered by a set of rows: last ts − first ts across the
/// rows that carry one. `null` when no row has a ts (e.g. an empty grid). Used
/// by the grid footer's `· <duration>` segment so live and timeline share one
/// derivation.
export function rowsDuration(rows: GridRow[]): number | null {
  let min = Infinity;
  let max = -Infinity;
  let any = false;
  for (const r of rows) {
    if (r.ts === null) continue;
    any = true;
    if (r.ts < min) min = r.ts;
    if (r.ts > max) max = r.ts;
  }
  return any ? max - min : null;
}

/// The database-client grid footer: `Showing N events · <duration>`, an
/// optional iteration label (`Iteration X of Y`), and — in the timeline —
/// Prev/Next paging through iterations. Structural only; values come from the
/// caller so this stays a pure presentation leaf shared by both run surfaces.
export function GridFooter({
  count,
  duration,
  iterLabel,
  paging,
}: {
  count: number;
  duration: string;
  iterLabel?: string;
  paging?: {
    onPrev: () => void;
    onNext: () => void;
    prevDisabled: boolean;
    nextDisabled: boolean;
  };
}) {
  return (
    <div className="grid-footer" role="status" aria-live="polite">
      <span className="grid-footer__count">
        Showing {count} {count === 1 ? "event" : "events"} · {duration}
      </span>
      {iterLabel && <span className="grid-footer__iter">{iterLabel}</span>}
      {paging && (
        <span className="grid-footer__paging">
          <button
            type="button"
            className="grid-footer__btn"
            onClick={paging.onPrev}
            disabled={paging.prevDisabled}
          >
            Prev
          </button>
          <button
            type="button"
            className="grid-footer__btn"
            onClick={paging.onNext}
            disabled={paging.nextDisabled}
          >
            Next
          </button>
        </span>
      )}
    </div>
  );
}
