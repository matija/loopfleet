# PRD: Agent Cockpit (working name — run naming workflow before launch)

A macOS desktop app (Tauri + Rust) for running looping coding agents against PRD-style plans, in sandboxed git worktrees, with a full timeline of what every run did.

Think: Codex-style project overview, but plan-centric instead of chat-centric. The PRD.md is the source of truth. Agents are consumers. The app runs zero AI inference itself — it supervises agent processes, normalizes their events, and shows the results.

Loops are based on the ralph-sandbox-exec pattern (github.com/matija/ralph-sandbox-exec): agents run in full-auto mode inside a macOS `sandbox-exec` (Seatbelt) profile. The OS sandbox is the single security boundary; per-agent permission systems are bypassed, not configured.

---

## Non-goals (v1)

- No AI features in the app itself. No summarization, no suggestions, no embedded models.
- No Linux/Windows. Architecture must not block them (sandbox and PTY layers behind traits), but nothing ships.
- No orchestrator mode (agent delegating to agent). Later runner variant.
- No auto-merge or PR automation. The app offers an explicit, user-targeted "use this run" that merges the chosen run's branch into a branch you name; it never auto-merges and never targets your main branch by default.
- No cloud, no accounts, no telemetry. Local-only.

---

## Supported agents (v1)

| Agent | Headless run transport | Interactive session transport | Tier |
|---|---|---|---|
| Claude Code | `claude -p --output-format stream-json` | same binary, `--input-format stream-json` (bidirectional JSONL/stdio) | v1.0 |
| pi | `pi --mode json` (AgentEvent JSONL) | `pi --mode rpc` (JSONL/stdio; split records on `\n` only) | v1.0 |
| cursor-agent | `cursor-agent -p --output-format stream-json` (one JSON per `\n`-terminated line; `--stream-partial-output` for token deltas; permission control via `--force`) | not yet mapped — resolve before M5 | v1.0 |
| Codex | `codex exec --json` (JSONL/stdio), resume via `codex exec resume` | `codex app-server` (JSON-RPC/stdio) | post-v1 |
| opencode | `opencode run` | `opencode serve` (HTTP + SSE, OpenAPI spec) | post-v1 |

Decision: build the adapter trait against all four protocols from day one (opencode's HTTP transport forces the trait to be transport-agnostic). Implement Claude Code, pi, and cursor-agent — the three agents actually run in daily loops — for v1.0. Codex and opencode land post-v1.

Decision: v1 is headless-only. Interactive session transports are deferred with M5 (plan chat); `open_session` stays in the trait signature but is unimplemented. cursor-agent's interactive transport is unmapped and must be resolved before M5.

Decision: only the headless column has to be solid for v1. Structured `stream-json`/JSONL output is confirmed for all three v1.0 agents, so the normalized event enum survives contact with reality.

---

## Architecture

```
┌────────────────────────── Tauri app ──────────────────────────┐
│  WebView UI                                                   │
│   projects · plans · runs timeline · diff viewer · plan chat  │
├───────────────────────────────────────────────────────────────┤
│  Rust core                                                    │
│   Supervisor ─ owns child processes, run loop, event log      │
│   AgentAdapter trait ─ 4 impls → normalized event enum        │
│   Sandbox trait ─ SeatbeltSandbox (renders .sb per run)       │
│   Git layer ─ git2 for reads; shell out for worktree ops      │
│   Store ─ SQLite (runs, iterations, events, refs)             │
└───────────────────────────────────────────────────────────────┘
```

### AgentAdapter

```rust
trait AgentAdapter {
    async fn start_run(&self, spec: &RunSpec) -> Result<RunHandle>;
    async fn open_session(&self, cwd: &Path, seed: SessionSeed) -> Result<SessionHandle>;
}
```

Both handles emit one normalized event enum, in two lanes. Everything downstream (timeline, diff capture, plan chat) consumes only this enum and never knows which agent produced it:

Adapter-sourced (mapped from the agent stream): `TurnStarted`, `AssistantText`, `Reasoning`, `ToolCall { call_id, name, input_excerpt }`, `ToolResult { call_id, ok, output_excerpt }`, `CommandRun { cmd, exit }`, `TurnCompleted { usage }`, `NeedsApproval`, `Failed { reason }`, `Ended`.

App-sourced (emitted by the app, not the adapter): `FileChanged { path }` — observed from worktree watching (git status / fs events), never parsed from the agent stream, so it is reliable across agents and catches files changed by shell commands too.

`ToolCall`/`ToolResult` are correlated by `call_id`. `CommandRun` is a deliberate normalization of the shell-exec tool (agents name it differently); all non-shell tools go through the generic `ToolCall`/`ToolResult` pair. `NeedsApproval` only fires in interactive sessions (M5); headless runs never surface it (see Sandbox).

This enum is the most load-bearing decision in the app. Get it right in M1; changing it later touches everything.

### Run pipeline

Plan task → create worktree → render `.sb` profile → spawn agent (structured output + full-auto flags) under `sandbox-exec` → normalize events → commit shadow ref per iteration → repeat N times → surface diff timeline.

The loop lives in the Rust supervisor, not in shell scripts. ralph-sandbox-exec is the reference implementation; the app absorbs its pattern (fresh context per iteration, plan file as durable state, `.sb` profile as boundary) because the scripts invoke agents in plain mode and lose the structured event stream.

### Sandbox

- `ralph.sb` becomes a template rendered per run with parameters injected.
- Write grants per run: the worktree path, the app-managed **per-run progress dir** (outside the repo, keyed by run-id — the agent reads and writes its progress file there), agent config/cache dirs, `/tmp`. **Not** the parent repo's `.git`: commits are app-owned (see Git layer), so the agent never needs `.git` write, which closes a real escape — an agent with `.git` write can plant a `hooks/` script or set `core.hooksPath`/`core.sshCommand` in `config` that later executes unsandboxed with the user's privileges.
- What the boundary actually is: **writes are confined to the worktree (+ progress dir); reads and network are not.** Agents need remote inference and tool network access, so egress is open, and `sandbox-exec` profiles leave `file-read*` broadly permitted because agent toolchains read from everywhere — meaning a sandboxed agent can read anything the user account can (`~/.ssh`, `~/.aws`, other repos' `.env`) and POST it out. State this plainly in the run UI's profile panel; overstating "sandboxed" is worse than stating the boundary narrowly. Read-scope confinement and an egress allowlist are post-v1 hardening.
- `sandbox-exec` is deprecated by Apple but remains the substrate everything builds on. Keep it behind the `Sandbox` trait; Linux gets Landlock/bubblewrap or containers later.
- Agents run with their own permissions disabled (`--dangerously-skip-permissions`, `--sandbox danger-full-access --ask-for-approval never`, cursor-agent `--force`, equivalents). No agent halts for approval in headless — cursor-agent fabricates a "skipped" answer, pi auto-resolves on timeout — so the Seatbelt profile is genuinely the only boundary. Show the active profile in the run UI — this is a trust feature, not a footnote.

### Git layer

- Worktree per run: `git worktree add`, branch `agent/<run-id>`.
- Shell out for worktree create/remove (libgit2 worktree support is patchy; the CLI is what agents expect). Use `git2` crate for reads: diffs, status, log.
- **Commits are app-owned.** The app (trusted, unsandboxed, via the git actor) snapshots worktree state to shadow ref `refs/agentapp/run-<id>/iter-<n>` after each iteration. The agent never runs `git commit` and never gets `.git` write. Cheap, real diffable history, never touches user branches.
- **One git actor:** all mutating git ops (worktree add/remove, shadow commits, ref updates) funnel through a single serialized task in `gitx` so concurrent runs never collide on git lockfiles. `git2` reads (diff, status, log) stay concurrent.
- Compare view: a diff viewer — diff-vs-diff of two (or more) runs' final refs on the same task. The app shows what each run produced; it never scores or judges. "Use this run" merges the chosen run's branch into a user-named target.

### Plans

- Convention: `PRD.md` at repo root, tasks as markdown checkboxes. Zero config.
- Alternative: user points at a folder (e.g. `plans/`) containing `.md` files. Each file is a plan.
- Parser extracts: title, task list (checkbox text + authored checked-state + a `{ normalized_text, line_hint }` anchor), free-form sections. Deterministic, no inference. The anchor's identity is the normalized text; the line is a hint/tiebreaker, not the key.
- **The PRD is frozen during runs** — neither the app nor the agent edits it. The authored `checked` state is input only (you may pre-check tasks to exclude them from launching); it is never a live progress signal.
- Task ↔ run binding: a run records which task it was launched from (Model B — one run, one task). Progress lives in a **per-run progress file, outside the repo, in an app-managed location keyed by run-id** (the sandbox grants the agent read+write there; the app injects the path via the run prompt). The agent writes what it did and, when finished, a machine-readable `STATUS: COMPLETE` marker for the bound task. The app watches that file; done = the marker appears. The app is read-only on both the PRD and the progress file.
- Per-task state in the plan view is **derived by the app from run records**, not read from any file (see Data model, `TaskStatus`): not-started / in-progress / completed-unaccepted / accepted. "Implemented" = a run you accepted via "use this run".

### Plan editing (chat) — deferred to M5, after the loop tool is in daily use

Interactive in-app plan editing is out of scope for the first usable build (which is headless-only). `open_session` stays in the `AgentAdapter` trait but is unimplemented; cursor-agent's interactive transport must be mapped before this ships. The rest of this section is the intended design, recorded for later.

- A plan chat is a `SessionHandle` rooted at the repo, with the plan file as seed context. The human types intent; the agent edits the markdown; a file watcher re-renders the plan live next to the chat.
- Agent is selectable per chat, **locked per session**. Switching agents ends the session and starts a fresh one with the new agent, seeded with the current plan file. Transcripts are not portable across agents; the plan file is the shared state, so nothing important is lost. The UI shows one continuous conversation with agent-switch markers.
- Sessions get full repo read access (the agent should read code to write a good plan). Write confinement is by convention plus the visible file-watcher diff, not by sandbox, in v1.
- `NeedsApproval` events surface as UI prompts in sessions. In headless runs the app neither surfaces nor gates approvals: no agent halts for one portably (cursor-agent fabricates a "skipped" answer, pi auto-resolves on timeout), so the Seatbelt profile is the only boundary and auto-resolved approvals are not tracked.

### Data model

```
Project   { id, repo_path, plan_convention }
Plan      { id, project_id, file_path, parsed_tasks[] }
Task      { plan_id, anchor: { normalized_text, line_hint }, text, checked }
          // `checked` is authored input only. Live per-task state is a DERIVED
          // TaskStatus (not-started | in-progress | completed-unaccepted | accepted),
          // computed from Run records, never stored as truth.
Run       { id, task_ref, agent, worktree_path, branch, sb_profile,
            progress_path (external, app-managed, keyed by run-id),
            max_iterations, status, accepted }
Iteration { run_id, n, shadow_ref, event_log_offset, usage, exit (agent process exit only) }
Session   { id, project_id, agent, plan_file, status }   // M5, deferred
Event     { seq, run_or_session_id, normalized_event_json, ts }
```

Run status machine: `queued → running → (completed | failed | stopped)`. `completed` = the agent wrote `STATUS: COMPLETE` for the bound task within N iterations; `failed` = N iterations reached still incomplete, or a crash. Acceptance (promoting a run's branch) is a separate flag, not a status. "Pause" is not offered in v1 — no agent can freeze mid-turn portably. Each run spawns in its own process group; stop = SIGTERM that group at the next iteration boundary (or immediately with confirmation), keep all shadow refs.

---

## Milestones

Build order for the first usable-for-me build: **M0 → M1 → M2 → M3 → M4 → (M6 hardening bits) → use it for a while → M5 → product polish (naming, domain, notarize).** M5 and the M6 release/distribution items are explicitly deferred until the loop tool has earned its keep.

Each task below is sized for one agent iteration. Run with ralph-sandbox-exec or, once M3 lands, with the app itself.

### M0 — Skeleton
- [x] Scaffold Tauri v2 app, Rust workspace with crates: `core`, `adapters`, `sandbox`, `gitx`, `store`
- [x] SQLite store with migrations for the data model above
- [x] Project registration: pick a folder, validate it is a git repo, persist

### M1 — Events and adapters
- [x] Define the normalized event enum (adapter-sourced vs app-sourced lanes; correlated `ToolCall`/`ToolResult`) + serde round-trip tests
- [x] `AgentAdapter` trait with `start_run` / `open_session` (latter unimplemented in v1); stub adapter that replays a fixture event log (used by all UI work)
- [x] Claude Code adapter, headless: spawn `claude -p --output-format stream-json`, map every event type to the enum, integration test against a fixture repo
- [x] pi adapter, headless: spawn `pi --mode json`, map AgentEvent JSONL to the enum, integration test
- [x] cursor-agent adapter, headless: spawn `cursor-agent -p --output-format stream-json`, map its stream to the enum, integration test
- [x] Event log writer: single-writer SQLite via a bounded channel (this IS the backpressure); `FileChanged` emitted here from worktree watching, not the adapters

### M2 — Sandbox and git
- [x] Port `ralph.sb` to a template; renderer that injects worktree path, agent dirs, `/tmp` (NB: parent-`.git` write is intentionally NOT granted — supersedes the stale wording; commits are app-owned per the Sandbox design section, so `.git` write is an escape vector, not a requirement)
- [x] `Sandbox` trait + `SeatbeltSandbox` impl wrapping command construction
- [x] Regression test: the agent CANNOT write the parent repo's `.git` under the rendered Seatbelt profile, while the app's out-of-sandbox git actor still commits (rewritten per the REVISIT note — the original ".git-grant succeeds" premise was removed when commits became app-owned)
- [x] Worktree manager: create/remove via git CLI, branch naming, orphan cleanup on startup
- [x] Shadow-ref snapshotter: commit worktree state to `refs/agentapp/run-<id>/iter-<n>` after each iteration
- [x] Diff service: iteration diff, run cumulative diff, run-vs-run diff (via git2)

### M3 — The loop
- [x] Supervisor: run lifecycle state machine, per-run process-group spawning, SIGTERM handling, one git actor serializing mutations
- [x] Iteration loop: N passes, fresh agent invocation each pass seeded with the bound task + prior progress file, app-owned snapshot between passes, stop conditions (bound task's `STATUS: COMPLETE`, N reached, failure)
- [x] Plan parser: PRD.md checkboxes (frozen; authored state only) + alternative plans-folder convention; text-based task anchors
- [x] Progress-file watcher: detect `STATUS: COMPLETE` in the per-run external progress file → mark run completed
- [ ] "Run N loops on task X" end-to-end against a fixture repo with the Claude Code adapter

### M4 — UI: overview and timeline
- [ ] Project list + plan view: rendered (frozen) PRD with a derived `TaskStatus` overlay, run-launch affordance per task; surface completed-unaccepted loudly (the review/compare queue)
- [ ] Run timeline: iterations as rows, per-iteration events, per-iteration diff viewer
- [ ] Live run view: streaming events, current file changes, stop button
- [ ] Compare view: two-or-more runs side by side, final-ref diffs, "use this run" → merge chosen branch into a user-named target

### M5 — Plan chat (deferred: build only after the loop tool is in daily use)
- [ ] Claude Code adapter, interactive: bidirectional stream-json session
- [ ] pi adapter, interactive: `--mode rpc` session; cursor-agent interactive (transport TBD); Codex app-server JSON-RPC (post-v1 agents)
- [ ] Chat UI with agent selector, session-locked, agent-switch = new seeded session with visual continuity
- [ ] Live plan pane: file watcher re-renders markdown beside the chat as the agent edits
- [ ] `NeedsApproval` prompt flow in sessions

### M6 — Hardening and release
- [ ] Orphan process reaping, crash recovery (mark running runs as failed on restart, keep refs)
- [ ] Agent binary discovery + version checks, graceful errors when a CLI is missing
- [ ] Settings: default agent, default iteration count, concurrency cap, sandbox profile overrides per project
- [ ] Codesign + notarize (Developer ID, existing Esploro pipeline), DMG build
- [ ] Run the naming workflow; register domain; replace working name

### Post-v1 (recorded, not planned)
- Codex adapter (`codex exec --json`), opencode adapter (HTTP/SSE, validates transport-agnostic trait)
- Interactive session transports (M5): Claude `--input-format stream-json`, pi `--mode rpc`, cursor-agent (transport TBD)
- Read-scope confinement + network egress allowlist (sandbox hardening)
- Daemon split: headless supervisor process so runs survive app restart
- Orchestrator runner variant
- Linux sandbox impl; Windows
- Advanced merge-back (cherry-pick / conflict assistance beyond the v1 one-click branch merge)

---

## Open questions

1. ~~Task ↔ run reconciliation conflict~~ — **resolved.** The PRD is frozen during runs and progress lives in a separate per-run file, so the agent never races the user on the PRD. Per-task state is derived from run records.
2. ~~"Done" detection per agent~~ — **resolved.** Under Model B a run is bound to one task; done = that run's progress file shows `STATUS: COMPLETE`. Uniform across agents, no per-adapter predicate.
3. **Two runs both "complete" on the same task** — **resolved for v1:** the compare view shows both diffs; "use this run" merges the chosen branch into a user-named target. The app never scores or auto-merges.
4. **Session write confinement (M5, deferred).** Plan chats can write anywhere in the repo (convention-bound only). Revisit when M5 is picked up; sandboxing sessions may break read-heavy tooling — test before deciding.

## Risks

- **The agent CLIs ship weekly.** Event schemas will drift. Mitigation: adapter integration tests pinned to CLI versions, run in CI; loud version-mismatch warnings in-app.
- **`sandbox-exec` deprecation.** Low near-term risk (Apple's own tooling ecosystem still depends on Seatbelt), but the `Sandbox` trait must stay honest — no Seatbelt details leaking above it.
- **Writes-only confinement is insufficient for untrusted plans/repos.** With reads and network open, a prompt-injected or malicious PRD/repo can read anything the user account can and exfiltrate it. Low risk while you only run your own plans; the moment you'd run someone else's, this boundary is not enough. Read-scope and egress hardening are post-v1.
- **Structured full-auto flag combinations are less traveled than interactive mode.** cursor-agent fabricates "user skipped" answers in headless (hapi #784); pi auto-resolves permission dialogs on timeout; Codex `--json` has open bugs with some features. Pin known-good flag sets per agent version, and remember no agent halts for approval — the sandbox is the boundary.
- **Crowded category** (Conductor, Crystal, Vibe Kanban). Differentiation is plan-centricity + OS-sandboxed full-auto + run comparison. If a competitor ships PRD-as-source-of-truth first, revisit positioning.
