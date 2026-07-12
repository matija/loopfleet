// The workbench tab strip: browser-style tabs above the main pane, one per open
// view (Welcome / plan / run / compare). Purely presentational — the tab model
// and reducer live in App.tsx; this renders items and reports focus/close.
//
// Each tab shows a kind icon + a truncated label + a close affordance; the
// active tab carries an accent. The strip never wraps: at a narrow window a
// full tab session scrolls horizontally rather than folding onto a second row
// (PRD: "at the 1200px window, 6+ tabs scroll rather than wrap").

export type TabKind = "welcome" | "plan" | "run" | "compare";

export type TabStripItem = {
  id: string;
  kind: TabKind;
  label: string;
};

export function TabStrip({
  tabs,
  activeId,
  onFocus,
  onClose,
}: {
  tabs: TabStripItem[];
  activeId: string;
  onFocus: (id: string) => void;
  onClose: (id: string) => void;
}) {
  return (
    <nav className="tab-strip" aria-label="Open views">
      {tabs.map((t) => {
        // Welcome is pinned home — always first, never closeable.
        const closeable = t.kind !== "welcome";
        return (
          <div
            key={t.id}
            className="tab-strip__tab"
            aria-current={t.id === activeId}
          >
            <button
              className="tab-strip__focus"
              onClick={() => onFocus(t.id)}
              title={t.label}
            >
              <TabIcon kind={t.kind} />
              <span className="tab-strip__label">{t.label}</span>
            </button>
            {closeable && (
              <button
                className="tab-strip__close"
                aria-label={`Close ${t.label}`}
                onClick={() => onClose(t.id)}
              >
                ×
              </button>
            )}
          </div>
        );
      })}
    </nav>
  );
}

// A small line icon per tab kind, mirroring the DB client's per-object glyphs:
// home for Welcome, a task list for a plan, a play triangle for a run, a
// split view for compare. Inline SVG keeps it dependency-free and themable via
// currentColor.
function TabIcon({ kind }: { kind: TabKind }) {
  const common = {
    className: "tab-strip__icon",
    width: 13,
    height: 13,
    viewBox: "0 0 16 16",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 1.4,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
    "aria-hidden": true,
  };
  switch (kind) {
    case "welcome":
      return (
        <svg {...common}>
          <path d="M2 7l6-5 6 5" />
          <path d="M4 6.5V13h8V6.5" />
        </svg>
      );
    case "plan":
      return (
        <svg {...common}>
          <path d="M5 4h8M5 8h8M5 12h8" />
          <path d="M2.5 4h.01M2.5 8h.01M2.5 12h.01" />
        </svg>
      );
    case "run":
      return (
        <svg {...common}>
          <path d="M5 3l8 5-8 5V3z" />
        </svg>
      );
    case "compare":
      return (
        <svg {...common}>
          <path d="M8 2v12" />
          <path d="M3 5h2M3 8h2M3 11h2M11 5h2M11 8h2M11 11h2" />
        </svg>
      );
  }
}
