// Presentational app shell: the sidebar now spans the full window height,
// with the main pane beside it (PRD M7: "sidebar (projects) / main pane
// layout"), plus a persistent bottom `dock` slot spanning the full width — the
// global run surface. It owns no data — callers pass the sidebar, main content,
// and dock as slots. The dock lives outside the scrolling main pane so it stays
// visible regardless of scroll.
//
// The sidebar's top strip (`.sidebar__top`) replaces the old full-width title
// bar and follows the app's own design (dark surface, app tokens) instead of
// the out-of-place native macOS title bar. The window is configured with
// `titleBarStyle: Overlay` + `hiddenTitle`, so the native traffic lights still
// work but sit over the strip's left inset; the strip carries the drag region
// that moves the window.

import type { ReactNode } from "react";
import { PanelLeftIcon, SettingsIcon, XIcon } from "./Icon";

export function AppShell({
  sidebar,
  children,
  dock,
  titlebarTrailing,
  notice,
  onOpenSettings,
  sidebarHidden,
  onToggleSidebar,
}: {
  sidebar: ReactNode;
  children: ReactNode;
  dock: ReactNode;
  /// Right-aligned content in the top window bar (the ⌘K entry point). Sits
  /// over the drag region but its buttons opt out of dragging.
  titlebarTrailing?: ReactNode;
  /// Optional dismissible accent notice shown above the Settings row. App.tsx
  /// owns whether there's anything to announce and how dismissal is tracked —
  /// AppShell just renders whatever it's given.
  notice?: { message: ReactNode; onDismiss: () => void } | null;
  /// Opens the overview view. Backs the pinned Settings row.
  onOpenSettings: () => void;
  /// Whether the sidebar column is collapsed to zero width. App.tsx owns the
  /// persisted state and the ⌘B shortcut; AppShell just renders the two
  /// mirrored toggle buttons (one in the strip, one in the main pane) so the
  /// sidebar is always recoverable.
  sidebarHidden: boolean;
  onToggleSidebar: () => void;
}) {
  return (
    <div className={`app-shell${sidebarHidden ? " app-shell--sidebar-hidden" : ""}`}>
      <aside
        className={`sidebar${sidebarHidden ? " sidebar--hidden" : ""}`}
        aria-hidden={sidebarHidden}
        inert={sidebarHidden ? true : undefined}
      >
        <div className="sidebar__top" data-tauri-drag-region>
          <div className="titlebar__brand-group" data-tauri-drag-region>
            <div className="titlebar__brand" data-tauri-drag-region>
              <span className="titlebar__mark" aria-hidden="true" />
              <span className="titlebar__name">loopfleet</span>
            </div>
            <button
              type="button"
              className="sidebar__collapse-btn"
              onClick={onToggleSidebar}
              title="Hide sidebar (⌘B)"
              aria-label="Hide sidebar"
              tabIndex={sidebarHidden ? -1 : 0}
            >
              <PanelLeftIcon size={15} />
            </button>
          </div>
          {titlebarTrailing}
        </div>
        {sidebar}
        <div className="sidebar__footer">
          {notice && (
            <div className="sidebar__notice">
              <span className="sidebar__notice-text">{notice.message}</span>
              <button
                type="button"
                className="sidebar__notice-dismiss"
                onClick={notice.onDismiss}
                aria-label="Dismiss"
              >
                <XIcon size={12} />
              </button>
            </div>
          )}
          <button
            type="button"
            className="sidebar__footer-row"
            onClick={onOpenSettings}
          >
            <SettingsIcon size={16} className="sidebar__footer-icon" />
            <span>Settings</span>
          </button>
        </div>
      </aside>
      <main className="main">
        {sidebarHidden && (
          <button
            type="button"
            className="main__restore-sidebar"
            onClick={onToggleSidebar}
            title="Show sidebar (⌘B)"
            aria-label="Show sidebar"
          >
            <PanelLeftIcon size={15} />
          </button>
        )}
        {children}
      </main>
      {dock}
    </div>
  );
}
