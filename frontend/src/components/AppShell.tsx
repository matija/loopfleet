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

export function AppShell({
  sidebar,
  children,
  dock,
  titlebarTrailing,
}: {
  sidebar: ReactNode;
  children: ReactNode;
  dock: ReactNode;
  /// Right-aligned content in the top window bar (the ⌘K entry point). Sits
  /// over the drag region but its buttons opt out of dragging.
  titlebarTrailing?: ReactNode;
}) {
  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="sidebar__top" data-tauri-drag-region>
          <div className="titlebar__brand" data-tauri-drag-region>
            <span className="titlebar__mark" aria-hidden="true" />
            <span className="titlebar__name">loopfleet</span>
          </div>
          {titlebarTrailing}
        </div>
        {sidebar}
      </aside>
      <main className="main">{children}</main>
      {dock}
    </div>
  );
}
