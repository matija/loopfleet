# PRD: Workbench UI — a database-client-style surface for loopfleet

loopfleet is a macOS desktop app (Tauri + Rust) that runs looping coding agents
against PRD-style plans in sandboxed git worktrees, with a full timeline of what
every run did. The Rust backend (M0–M6) and the first React frontend (M7) are
done; the v1 build PRD is archived at `prds/agent-cockpit-v1.md`.

M7 shipped a working-but-flat React app: a projects-only sidebar
(`App.tsx`), one main pane that swaps between three mutually-exclusive views
(plan / live run / timeline / compare) driven by `selectedRun` and
`compareTarget`, and a bottom run dock. It works, and this plan keeps that
single-view model: the main pane shows exactly one view at a time, with the
sidebar's plan tree and the bottom run dock as the always-present navigators and
an in-view "← Back" control returning to the project's plan.

This plan reshapes that surface into a **database-client-style workbench**
modeled on a modern database client: a connections-style sidebar, a filterable
object tree with counts, a per-view command bar, typed data grids with enum
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
with per-object **row counts**; a **per-view command bar** (object-name
pill, a `WHERE …` filter, a **Run** button, a **Connected** status pill, an "Xs
ago" freshness stamp); **Data / Privileges** subviews; a **typed data grid** (row
numbers, column headers carrying a type badge + PK/FK icons, **enum values
rendered as colored pills**, `NULL` as a muted pill); a **footer** (`Showing
1–200 of ~522 rows · 150 ms`, Prev/Next); a **top bar** drawn in the app's own
design with global **⌘K search** and an environment badge.

## Domain mapping — what each borrowed feature becomes

| Database client | loopfleet |
|---|---|
| Connection (status dot, `user@host`) | **Project** — repo name + short path; dot lit when it has active runs |
| Object list with row counts | **Plan tree** — tasks with run-count badges; a "Runs" group |
| Per-view command bar | **Run action bar** — task pill + event filter + Run/Re-run + agent "Connected" pill + live "Xs ago" |
| Data / Privileges subviews | **Run subviews** — Events / Diff / Files |
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
  tokens only where a genuinely new pattern needs one (grid, palette).
- **No editable grids.** Cells are read-only — loopfleet is read-only on both the
  PRD and the progress file.
- **No browser-style multi-view strip.** The main pane shows one view at a
  time. Opening a task / run / compare replaces the current view; the sidebar
  and dock remain the persistent navigators and an in-view "← Back" returns to
  the project's plan. This drops an earlier direction that surfaced
  inconsistent, unexpected views.

---

## Tasks (each sized for one agent iteration)

- [x] **View model.** A single `View` union (`overview | plan | task | run |
  compare`) in `App.tsx`, replacing the M7 mutually-exclusive
  `selectedRun` / `compareTarget` switch. Selecting a project opens its plan;
  opening a task / run / compare replaces the current view; a dismissed run or
  the in-view "← Back" returns to the selected project's plan (or the overview
  when no project is selected). → verify: opening a run from the dock takes the
  main pane; Back returns to the plan.
- [x] **Sidebar as connections.** Restyle `project-item` into a connection row:
  a status dot (accent when the project has an active run, faint otherwise), repo
  name + short-path subtitle; move add-project to a header "+" button; add a
  "filter tables…"-style live filter input over projects/tasks. → verify: the
  filter narrows the list live; the dot lights while a run is active.
- [x] **Plan tree with counts.** Under the selected project, render tasks as a
  filterable list with a right-aligned **run-count badge** (from `plan_overview`),
  grouped like the DB object tree; clicking a task opens it in the main pane.
  Keep the derived `TaskStatus` badge; surface completed-unaccepted loudly (the
  review queue). → verify: counts match the overview; a click opens the task.
- [x] **Per-view command bar.** A `CommandBar` for run/task views: a task-name
  pill, a `WHERE …`-style client-side **event filter**, the **Run / Re-run**
  control (agent + iterations), an agent **status pill** ("Connected" / "missing"
  from `agent_status`), and a live **"Xs ago"** stamp on the active run. This
  relocates the launch control out of the plan body. → verify: the filter narrows
  the event grid; launch still works from the bar.
- [x] **Typed event grid + enum pills.** Replace the live/timeline event list with
  a reusable `DataGrid`: row numbers, columns (`seq`, `type`, `detail`, `ts`), the
  `type` column rendering each `NormalizedEvent` as a **colored enum pill** (stable
  color per variant), empties as a muted `NULL`-style pill. Reuse it in
  `LiveRunView` and `RunTimeline`. → verify: every event variant maps to a
  distinct labeled pill; a live stream appends rows.
- [x] **Run subviews (Data / Privileges analog).** Inside a run view, an **Events
  / Diff / Files** subview bar: Diff hosts the existing per-iteration diff/patch
  viewer, Files the changed-files list, Events the grid above. → verify:
  switching subviews preserves scroll and the run subscription.
- [x] **Grid footer.** A footer under the grid: `Showing N events · <duration>`,
  the iteration count, and — in the timeline — a Prev/Next for iteration paging.
  → verify: counts and duration match the timeline data.
- [ ] **⌘K command palette.** A global palette (`Cmd/Ctrl-K`) that fuzzy-searches
  projects, tasks, and runs and opens the match in the main pane, plus quick
  actions (add project, open settings). → verify: keyboard-only open → navigate
  → select; Esc closes.
- [x] **Top window bar.** A slim top bar drawn in the app's own design (dark
  surface, app tokens) replacing the out-of-place native macOS title bar. The
  window uses an overlay title bar so the native traffic lights still work over
  it; the bar carries the drag region and a centered brand, and will host the ⌘K
  entry point and an environment badge. → verify: the bar reads as part of the
  app, not the OS; dragging the bar moves the window.
- [ ] **Polish + parity pass.** Empty / loading / error states for every new
  surface; keyboard focus order across views and palette; responsive to 1200×800;
  confirm the overview reads as an intentional home, not a blank pane. Remove
  only the M7 code the view model orphans (its own orphans, per surgical-changes).
  → verify: a cold open with no projects and a full session both read as
  intentional; no dead imports.

---

## Success criteria

From the existing React app: opening a task, run, or compare takes the main pane
with a single, consistent view and an in-view Back control; the sidebar reads as
a connections panel with a live filter and run-count badges; each run view has a
command bar with a working event filter and launch control; events render in a
typed grid with colored enum pills and a counts/timing footer; ⌘K opens any
project, task, or run. The top window bar follows the app's own dark design.
Backend crates and every Tauri command signature are byte-for-byte unchanged.
