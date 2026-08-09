// The shell's single per-view strip: sits above the main pane's body, one per
// view, carrying a breadcrumb at the left, a filter/status cluster in the
// middle, and actions at the right. It's part of the drag region (like
// `.sidebar__top`) since it sits under the window's top edge, but its slots
// opt out individually so their controls stay clickable.

import type { ReactNode } from "react";

// `filterRef`/`actionsRef` resolve to the `.toolbar__filter`/`.toolbar__actions`
// DOM nodes themselves, so the active view's owning component can portal its
// task pill/filter/status cluster and its primary action button(s) straight
// into the strip — the state/handlers stay where they're defined; only the
// rendered DOM relocates. See TaskTab/LiveRunView/RunTimeline/CompareView's
// `toolbarActions` prop and LiveRunView's `toolbarFilter` prop.
export function Toolbar({
  breadcrumb,
  filterRef,
  actionsRef,
  actions,
  trailing,
}: {
  /// Left-aligned: current view's path (e.g. project / plan / task).
  breadcrumb?: ReactNode;
  /// Portal target for a view's task pill / event filter / freshness / agent
  /// status cluster (LiveRunView only, currently).
  filterRef?: (el: HTMLDivElement | null) => void;
  /// Portal target for right-aligned view-specific actions (buttons, chips).
  actionsRef?: (el: HTMLDivElement | null) => void;
  actions?: ReactNode;
  /// Trailing icon-button slot, e.g. an overflow or panel toggle.
  trailing?: ReactNode;
}) {
  return (
    <div className="toolbar" data-tauri-drag-region>
      <div className="toolbar__breadcrumb">{breadcrumb}</div>
      <div className="toolbar__filter" ref={filterRef} />
      <div className="toolbar__actions" ref={actionsRef}>
        {actions}
      </div>
      {trailing && <div className="toolbar__trailing">{trailing}</div>}
    </div>
  );
}
