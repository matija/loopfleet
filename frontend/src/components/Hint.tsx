// Explanatory copy attached to the control it explains, disclosed in two
// steps: a one-line summary that is always visible, and the full multi-
// sentence text behind an info affordance beside it.
//
// The reason for the split is layout, not tidiness. A paragraph rendered
// inline under a control grows and shrinks with its own content — three
// sentences push every control below it down the panel, and a mode change
// that swaps one explanation for a longer one shifts the page under the
// user's cursor. The summary here is a single line, so the space the hint
// occupies is fixed; the detail opens in a Popover, which portals out of the
// panel and therefore costs the layout nothing however long it runs.
//
// The affordance is a button, not a hover target: a hover-only tooltip is
// unreachable by keyboard and untappable by touch, and this text is the
// explanation of what a setting *does*, not a decorative aside.

import { useRef, useState, type ReactNode } from "react";
import { Popover } from "./Popover";
import { InfoIcon } from "./Icon";

export function Hint({
  summary,
  label,
  children,
}: {
  /// The always-visible line. Keep it to one line at the panel's usual width
  /// — it's the part that has to fit next to the controls, not the detail.
  summary: string;
  /// Names the subject for the info button's accessible label and the
  /// detail panel's, e.g. "worktree deletion" -> "More about worktree
  /// deletion". Never shown on screen.
  label: string;
  /// The full explanation. Rendered inside the popover, so it may be as long
  /// as it needs to be.
  children: ReactNode;
}) {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);

  return (
    <div className="hint">
      <p className="hint__summary">{summary}</p>
      <button
        ref={anchorRef}
        className="hint__more"
        aria-label={`More about ${label}`}
        aria-expanded={open}
        onClick={() => setOpen((was) => !was)}
      >
        <InfoIcon size={14} />
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={anchorRef}
        role="dialog"
        aria-label={`About ${label}`}
        className="hint__detail"
      >
        {children}
      </Popover>
    </div>
  );
}
