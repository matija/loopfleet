// The shell's single per-view strip: sits above the main pane's body, one per
// view, carrying a breadcrumb at the left and actions at the right. It's part
// of the drag region (like `.sidebar__top`) since it sits under the window's
// top edge, but its slots opt out individually so their controls stay
// clickable.

import { forwardRef, type ReactNode } from "react";

// `ref` resolves to the `.toolbar__actions` DOM node itself, so the active
// view's owning component can portal its primary action button(s) straight
// into the strip — the button's state/handlers stay where they're defined;
// only the rendered DOM relocates. See TaskTab/LiveRunView/RunTimeline/
// CompareView's `toolbarActions` prop.
export const Toolbar = forwardRef<
  HTMLDivElement,
  {
    /// Left-aligned: current view's path (e.g. project / plan / task).
    breadcrumb?: ReactNode;
    /// Right-aligned view-specific actions (buttons, chips).
    actions?: ReactNode;
    /// Trailing icon-button slot, e.g. an overflow or panel toggle.
    trailing?: ReactNode;
  }
>(function Toolbar({ breadcrumb, actions, trailing }, actionsRef) {
  return (
    <div className="toolbar" data-tauri-drag-region>
      <div className="toolbar__breadcrumb">{breadcrumb}</div>
      <div className="toolbar__actions" ref={actionsRef}>
        {actions}
      </div>
      {trailing && <div className="toolbar__trailing">{trailing}</div>}
    </div>
  );
});
