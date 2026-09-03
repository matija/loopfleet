# PRD: Workbench UI — a panel-style workbench surface for Loopfleet

Loopfleet is a macOS desktop app (Tauri + Rust) that runs looping coding agents
against PRD-style plans in sandboxed git worktrees, with a full timeline of what
every run did.

**This is a frontend-only milestone**: no Rust command signatures change.

---

## Non-goals

- **No backend changes.** Backend crates and command signatures stay unchanged.
- **No query language.** The `WHERE …` analog is a plain client-side filter.

An authored plan may show a checkbox inside an example fence; it is not a task:

```markdown
- [ ] **Not a task.** Illustrative only.
```

---

## Tasks (each sized for one agent iteration)

- [x] **View model.** A single `View` union (`overview | plan | task | run |
  compare`) in `App.tsx`, replacing the mutually-exclusive
  `selectedRun` / `compareTarget` switch. → verify: opening a run from the dock
  takes the main pane; Back returns to the plan.
- [x] **Sidebar as connections.** Restyle `project-item` into a connection row:
  a status dot, repo name + short-path subtitle; add a live filter input over
  projects/tasks. → verify: the filter narrows the list live.
- [ ] **Typed event grid + enum pills.** Replace the event list with a reusable
  `DataGrid`: row numbers, columns (`seq`, `type`, `detail`, `ts`), the `type`
  column rendering each `NormalizedEvent` as a colored enum pill, empties as a
  muted `NULL`-style pill. → verify: every event variant maps to a distinct
  labeled pill; a live stream appends rows.
- [ ] **⌘K command palette.** A global palette that fuzzy-searches projects,
  tasks, and runs. → verify: keyboard-only open → navigate → select; Esc closes.

---

## Success criteria

Events render in a typed grid with colored enum pills; ⌘K opens any project,
task, or run. Every Tauri command signature is byte-for-byte unchanged.
