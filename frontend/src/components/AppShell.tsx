// Presentational app shell: the fixed sidebar / main-pane grid every view lives
// inside (PRD M7: "sidebar (projects) / main pane layout"), plus a persistent
// bottom `dock` slot spanning the full width — the global run surface. It owns
// no data — callers pass the sidebar, main content, and dock as slots. The dock
// lives outside the scrolling main pane so it stays visible regardless of scroll.

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
      <aside className="sidebar">
        <div className="sidebar__brand">
          <h1>loopfleet</h1>
          <span>agent cockpit</span>
        </div>
        {sidebar}
      </aside>
      <main className="main">{children}</main>
      {dock}
    </div>
  );
}
