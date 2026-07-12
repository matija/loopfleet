// Presentational app shell: the fixed sidebar / main-pane grid every view lives
// inside (PRD M7: "sidebar (projects) / main pane layout"). It owns no data —
// callers pass the sidebar and main content as slots. Later M7 tasks (projects
// component, plan view, run surfaces) render into these slots.

import type { ReactNode } from "react";

export function AppShell({
  sidebar,
  children,
}: {
  sidebar: ReactNode;
  children: ReactNode;
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
    </div>
  );
}
