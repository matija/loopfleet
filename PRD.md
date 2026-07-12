# PRD: Workbench UI — a tabbed, database-client-style surface for loopfleet

loopfleet is a macOS desktop app (Tauri + Rust) that runs looping coding agents
against PRD-style plans in sandboxed git worktrees, with a full timeline of what
every run did. The Rust backend (M0–M6) and the first React frontend (M7) are
done; the v1 build PRD is archived at `prds/agent-cockpit-v1.md`.

M7 shipped a working-but-flat React app: a projects-only sidebar
(`App.tsx`), one main pane that swaps between three mutually-exclusive views
(plan / live run / timeline / compare) driven by `selectedRun` and
`compareTarget`, and a bottom run dock. It works, but it navigates like a wizard,
not a workbench — you can only look at one thing at a time. Watching two runs, or
holding a run's diff next to its task, means losing your place.

This plan reshapes that surface into a **tabbed workbench** modeled on a modern
database client: a connections-style sidebar, a filterable object tree with
counts, browser-style tabs, a per-tab command bar, typed data grids with enum
badges, and ⌘K. The domain fit is exact — loopfleet's normalized event enum and
derived `TaskStatus` are the natural analog of a DB client's typed columns and
enum values.

**This is a frontend-only milestone**, the same constraint as M7: no Rust command
signatures change. Every task consumes the existing command surface
(`plan_overview`, `run_timeline`, `compare_task`, `launch_run`, `stop_run`,
`use_run`, `agent_status`, settings). If a view wants data the backend doesn't
expose, note it — don't silently widen the command surface.

---

## Reference — the interface being borrowed from

A modern database client with: a **connections sidebar** (status dot, `name` +
`user@host` subtitle, a "+" to add, a filter box); a **filterable object list**
with per-object **row counts**; **browser-style tabs** across the top (icon +
label + close, a persistent "Welcome" tab); a **per-tab command bar** (object-name
pill, a `WHERE …` filter, a **Run** button, a **Connected** status pill, an "Xs
ago" freshness stamp); **Data / Privileges** subtabs; a **typed data grid** (row
numbers, column headers carrying a type badge + PK/FK icons, **enum values
rendered as colored pills**, `NULL` as a muted pill); a **footer** (`Showing
1–200 of ~522 rows · 150 ms`, Prev/Next); a **top bar** with global **⌘K search**
and an environment badge.

## Domain mapping — what each borrowed feature becomes

| Database client | loopfleet |
|---|---|
| Connection (status dot, `user@host`) | **Project** — repo name + short path; dot lit when it has active runs |
| Object list with row counts | **Plan tree** — tasks with run-count badges; a "Runs" group |
| Browser tabs + Welcome tab | **Open views** — each task / run / compare / timeline is a closeable tab; a pinned Welcome home |
| Per-tab command bar | **Run action bar** — task pill + event filter + Run/Re-run + agent "Connected" pill + live "Xs ago" |
| Data / Privileges subtabs | **Run subtabs** — Events / Diff / Files |
| Typed grid + enum pills | **Event/iteration grid** — normalized event types as colored pills (`ToolCall`, `CommandRun`, `AssistantText`, …); empty as a muted `NULL`-style pill |
| Footer counts + timing | iteration/event counts + run duration |
| ⌘K search | command palette across projects, tasks, runs |
| Environment badge | active default agent / sandbox-profile note |

---

## Non-goals

- **No backend changes.** No new or altered Tauri commands (same as M7). Backend
  crates and command signatures stay byte-for-byte unchanged.
- **No SQL / query language.** The `WHERE …` analog is a plain client-side filter
  over the already-loaded events/tasks, not a query surface.
- **No new theme system.** Reuse the existing `tokens.css` dark palette; add
  tokens only where a genuinely new pattern needs one (tab strip, grid, palette).
- **No editable grids.** Cells are read-only — loopfleet is read-only on both the
  PRD and the progress file.

---

## Tasks (each sized for one agent iteration)

- [x] **Tab model.** Introduce a `WorkbenchTab` union (`welcome | plan | run |
  compare`) and a tab store in `App.tsx`, replacing the mutually-exclusive
  `selectedRun` / `compareTarget` switch. Opening a task/run/compare pushes or
  focuses a tab; tabs are closeable; a pinned "Welcome" tab is always first.
  → verify: opening two runs keeps both as switchable tabs; closing one falls
  back to a neighbor.
- [x] **Tab strip.** A `TabStrip` component above the main pane: per-tab icon
  (by kind) + label + close affordance, active-tab accent, horizontal overflow
  scroll — matching the reference's tab styling. New `tabs.css`; add tokens as
  needed. → verify: at the 1200px window, 6+ tabs scroll rather than wrap.
- [ ] **Sidebar as connections.** Restyle `project-item` into a connection row:
  a status dot (accent when the project has an active run, faint otherwise), repo
  name + short-path subtitle; move add-project to a header "+" button; add a
  "filter tables…"-style live filter input over projects/tasks. → verify: the
  filter narrows the list live; the dot lights while a run is active.
- [ ] **Plan tree with counts.** Under the selected project, render tasks as a
  filterable list with a right-aligned **run-count badge** (from `plan_overview`),
  grouped like the DB object tree; clicking a task opens/focuses its tab. Keep the
  derived `TaskStatus` badge; surface completed-unaccepted loudly (the review
  queue). → verify: counts match the overview; a click opens a tab.
- [ ] **Per-tab command bar.** A `CommandBar` for run/task tabs: a task-name pill,
  a `WHERE …`-style client-side **event filter**, the **Run / Re-run** control
  (agent + iterations), an agent **status pill** ("Connected" / "missing" from
  `agent_status`), and a live **"Xs ago"** stamp on the active run. This relocates
  the launch control out of the plan body. → verify: the filter narrows the event
  grid; launch still works from the bar.
- [ ] **Typed event grid + enum pills.** Replace the live/timeline event list with
  a reusable `DataGrid`: row numbers, columns (`seq`, `type`, `detail`, `ts`), the
  `type` column rendering each `NormalizedEvent` as a **colored enum pill** (stable
  color per variant), empties as a muted `NULL`-style pill. Reuse it in
  `LiveRunView` and `RunTimeline`. → verify: every event variant maps to a
  distinct labeled pill; a live stream appends rows.
- [ ] **Run subtabs (Data / Privileges analog).** Inside a run tab, an **Events /
  Diff / Files** subtab bar: Diff hosts the existing per-iteration diff/patch
  viewer, Files the changed-files list, Events the grid above. → verify: switching
  subtabs preserves scroll and the run subscription.
- [ ] **Grid footer.** A footer under the grid: `Showing N events · <duration>`,
  the iteration count, and — in the timeline — a Prev/Next for iteration paging.
  → verify: counts and duration match the timeline data.
- [ ] **⌘K command palette.** A global palette (`Cmd/Ctrl-K`) that fuzzy-searches
  projects, tasks, and runs and opens the match as a tab, plus quick actions (add
  project, open settings). → verify: keyboard-only open → navigate → select; Esc
  closes.
- [ ] **Top bar + environment badge.** A slim top bar in `AppShell` with the ⌘K
  entry point and an environment badge showing the default agent / active
  sandbox-profile note (tying into the honest sandbox-boundary framing the v1 PRD
  calls a trust feature). → verify: the badge reflects `settings` / `agent_status`;
  the boundary statement stays reachable.
- [ ] **Polish + parity pass.** Empty / loading / error states for every new
  surface; keyboard focus order across tabs and palette; responsive to 1200×800;
  confirm the Welcome tab reads as an intentional home, not a blank pane. Remove
  only the M7 code the tab model orphans (its own orphans, per surgical-changes).
  → verify: a cold open with no projects and a full-tab session both read as
  intentional; no dead imports.

---

## Success criteria

From the existing React app: opening several tasks/runs yields independent,
switchable, closeable tabs with a persistent Welcome home; the sidebar reads as a
connections panel with a live filter and run-count badges; each run tab has a
command bar with a working event filter and launch control; events render in a
typed grid with colored enum pills and a counts/timing footer; ⌘K opens any
project, task, or run. Backend crates and every Tauri command signature are
byte-for-byte unchanged.
