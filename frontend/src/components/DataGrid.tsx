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
      return { label: "TurnStarted", tone: "neutral" };
    case "assistant_text":
      return { label: "AssistantText", tone: "text" };
    case "reasoning":
      return { label: "Reasoning", tone: "reasoning" };
    case "tool_call":
      return { label: "ToolCall", tone: "tool" };
    case "tool_result":
      return { label: "ToolResult", tone: e.ok ? "tool" : "error" };
    case "command_run":
      return { label: "CommandRun", tone: "command" };
    case "turn_completed":
      return { label: "TurnCompleted", tone: "neutral" };
    case "needs_approval":
      return { label: "NeedsApproval", tone: "warn" };
    case "failed":
      return { label: "Failed", tone: "error" };
    case "ended":
      return { label: "Ended", tone: "neutral" };
    case "file_changed":
      return { label: "FileChanged", tone: "file" };
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
    default:
      return "";
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
        <span className="data-grid__cell data-grid__cell--num">#</span>
        <span className="data-grid__cell data-grid__cell--seq">seq</span>
        <span className="data-grid__cell data-grid__cell--type">type</span>
        <span className="data-grid__cell data-grid__cell--detail">detail</span>
        <span className="data-grid__cell data-grid__cell--ts">ts</span>
      </div>
      {rows.map((r, i) => {
        const pill = eventPill(r.event);
        const detail = eventDetail(r.event);
        const ts = formatTs(r.ts);
        return (
          <div className="data-grid__row" role="row" key={r.seq}>
            <span className="data-grid__cell data-grid__cell--num">{i + 1}</span>
            <span className="data-grid__cell data-grid__cell--seq">{r.seq}</span>
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
