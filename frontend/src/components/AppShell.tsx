// Presentational app shell: a custom top window bar, the fixed sidebar / main-
// pane grid every view lives inside (PRD M7: "sidebar (projects) / main pane
// layout"), plus a persistent bottom `dock` slot spanning the full width — the
// global run surface. It owns no data — callers pass the sidebar, main content,
// and dock as slots. The dock lives outside the scrolling main pane so it stays
// visible regardless of scroll.
//
// The top window bar follows the app's own design (dark surface, app tokens)
// instead of the out-of-place native macOS title bar. The window is configured
// with `titleBarStyle: Overlay` + `hiddenTitle`, so the native traffic lights
// still work but sit over our bar; the bar carries the drag region that moves
// the window, with the brand centered where it never collides with the lights.

import type { ReactNode } from "react";

export function AppShell({
  sidebar,
  children,
  dock,
}: {
  sidebar: ReactNode;
  children: ReactNode;
  dock: ReactNode;
}) {
  return (
    <div className="app-shell">
      <div className="titlebar" data-tauri-drag-region>
        <div className="titlebar__brand" data-tauri-drag-region>
          <span className="titlebar__name">loopfleet</span>
          <span className="titlebar__sub">agent cockpit</span>
        </div>
      </div>
      <aside className="sidebar">{sidebar}</aside>
      <main className="main">{children}</main>
      {dock}
    </div>
  );
}
