// Hand-maintained mirror of the Rust command payloads (PRD M7 decision: no
// codegen in v1 — the surface is small and stable). Every type here corresponds
// to a `serde::Serialize` struct/enum in the `store`, `core`, or `adapters`
// crates, or to a payload the `src-tauri` command layer emits. Field names match
// the Rust `serde` output (snake_case); the source struct is noted per type. If
// a view needs data the backend does not expose, add a note here — do not widen
// the Rust command surface silently.

// --- store: projects.rs ---

/// A registered project. `store::Project`.
export type Project = {
  id: string;
  /// Absolute, canonicalized repo path. Unique per project.
  repo_path: string;
  /// `"prd"` (PRD.md at root) or `"folder"` (plans/ dir).
  plan_convention: string;
};

// --- store: settings.rs ---

/// Global app defaults. `store::Settings`.
export type Settings = {
  default_agent: string;
  default_iterations: number;
  /// Max simultaneously active runs; `0` means no cap.
  concurrency_cap: number;
  /// Hours a finished run's worktree survives before the sweep reaps it.
  /// `0` means immediately, `-1` means never (accepted runs are always
  /// swept regardless).
  worktree_retention_hours: number;
};

// --- adapters: discovery.rs ---

/// One agent CLI's discovery result. `adapters::AgentStatus`.
export type AgentStatus = {
  key: string;
  display: string;
  binary: string;
  tested_version: string;
  /// Found on PATH and ran.
  installed: boolean;
  /// Detected version, if installed and recognized.
  version: string | null;
  /// `true`/`false` once installed: does the detected version match the tested
  /// one? `null` when not installed or unrecognized.
  version_matches: boolean | null;
  /// Reason when not installed, or a note when the version wasn't recognized.
  detail: string | null;
};

// --- core: usage.rs ---

/// How a usage snapshot's numbers were come by. `core::UsageSource`
/// (snake_case).
export type UsageSource =
  /// The agent reported the figure itself.
  | "reported"
  /// Derived from an observed rate-limit notice — the agent said it was
  /// blocked, not how much of the window it had spent.
  | "inferred"
  /// Nothing is known. Never to be rendered as headroom.
  | "unknown";

/// One agent's limit consumption at a point in time. `core::UsageSnapshot`.
///
/// Every agent's limit reporting collapses into this shape, so the UI never
/// learns the per-agent dialects. Instants are epoch millis.
export type UsageSnapshot = {
  /// Which agent this describes — matches `AgentStatus.key`.
  agent_key: string;
  /// The model the limit applies to, when the agent scopes limits per model.
  model: string | null;
  /// The limit window as the agent names it (e.g. `"5h"`, `"weekly"`).
  limit_window: string | null;
  /// Fraction of the window consumed, always in `0.0..=1.0`.
  used_fraction: number;
  /// When the window resets, epoch millis, when known.
  reset_at_ms: number | null;
  /// When this snapshot was observed, epoch millis.
  observed_at_ms: number;
  /// How much of the above is the agent's word. A `used_fraction` of `0` with
  /// source `"unknown"` means "no idea", not "plenty left".
  source: UsageSource;
};

// --- core: task_status.rs / overview.rs ---

/// Derived per-task state (kebab-case, from `core::TaskStatus`).
export type TaskStatus =
  | "not-started"
  | "in-progress"
  | "completed-unaccepted"
  | "accepted";

/// One task with authored fields plus the app-derived live state.
/// `core::overview::TaskView`.
export type TaskView = {
  /// The stable anchor identity — what a launched run binds to.
  anchor: string;
  line_hint: number;
  text: string;
  /// Authored `- [x]` state: the "implemented" baseline — read as `Accepted`
  /// by the derived status when no outranking run exists. Still runnable;
  /// launching is never gated by it.
  checked: boolean;
  status: TaskStatus;
  /// How many runs are bound to this task.
  run_count: number;
};

/// One plan rendered for the overview. `core::overview::PlanView`.
export type PlanView = {
  plan_id: string;
  file_path: string;
  title: string | null;
  /// The raw plan file, for the UI to render the frozen PRD verbatim.
  markdown: string;
  tasks: TaskView[];
};

// --- store: runs.rs ---

/// A run's lifecycle token (`runs.status`, from `core::RunState::as_str`).
export type RunStatus =
  | "queued"
  | "running"
  | "completed"
  | "failed"
  | "stopped"
  /// The agent hit a rate limit and the run ended early to wait it out. Terminal
  /// like the rest; the reset time rides the `rate_limited` event, not the token.
  | "limit-reached";

/// A run's bearing on its task's status. `store::RunSummary`.
export type RunSummary = {
  id: string;
  task_anchor: string;
  status: RunStatus;
  accepted: boolean;
};

// --- core: timeline.rs ---

/// One file's change in a diff. `core::timeline::FileChangeView`.
/// `status` is `core::gitx::ChangeStatus` stringified (e.g. "added",
/// "modified", "deleted", "renamed", "copied", "other").
export type FileChangeView = {
  path: string;
  old_path: string | null;
  status: string;
  insertions: number;
  deletions: number;
};

/// A diff: per-file summary plus the full unified patch. `core::timeline::DiffView`.
export type DiffView = {
  files: FileChangeView[];
  patch: string;
};

/// One normalized event with its log position and timestamp.
/// `core::timeline::TimelineEvent`.
export type TimelineEvent = {
  seq: number;
  ts: number;
  event: NormalizedEvent;
};

/// One iteration row. `core::timeline::IterationView`.
export type IterationView = {
  n: number;
  shadow_ref: string | null;
  events: TimelineEvent[];
  diff: DiffView | null;
};

/// A whole run's timeline. `core::timeline::RunTimeline`.
export type RunTimeline = {
  run_id: string;
  agent: string;
  status: RunStatus;
  task_anchor: string;
  max_iterations: number;
  /// Whether the run was accepted ("use this run"). Separate from `status`: a
  /// completed run may or may not have been merged into a branch.
  accepted: boolean;
  /// The run's isolated worktree, or `null` once the sweep has reclaimed it
  /// (the diff and report remain available regardless).
  worktree_path: string | null;
  iterations: IterationView[];
};

// --- core: compare.rs ---

/// One run in the compare view. `core::compare::RunCompare`.
export type RunCompare = {
  run_id: string;
  agent: string;
  status: RunStatus;
  accepted: boolean;
  /// The run's final iteration shadow ref (`null` if it produced no snapshot).
  final_ref: string | null;
  /// What the run produced against its base (`null` if unreadable).
  diff: DiffView | null;
  /// The run's isolated worktree, or `null` once the sweep has reclaimed it
  /// (the diff and report remain available regardless).
  worktree_path: string | null;
};

/// The runs competing on one task. `core::compare::CompareView`.
export type CompareView = {
  task_anchor: string;
  runs: RunCompare[];
};

// --- src-tauri: use_run ---

/// The result of "use this run". `src-tauri::UseRunResult`.
export type UseRunResult = {
  target_branch: string;
  /// The squashed commit created on the target branch (an up-to-date merge
  /// reports the target's existing tip) — safe to show to the user.
  merged_commit: string;
  created: boolean;
  up_to_date: boolean;
};

// --- src-tauri: sweep_worktrees_now ---

/// The result of an on-demand worktree sweep, returned by
/// `sweep_worktrees_now`. `src-tauri::SweepResult`.
export type SweepResult = {
  removed: number;
  bytes_reclaimed: number;
};

// --- src-tauri: plan_edit ---

/// A proposed AI edit to a plan document, returned by `plan_edit`. The default
/// agent ran one pass in an isolated worktree; the UI renders `original` vs
/// `proposed` as a reviewable diff and lands or drops it via `plan_edit_apply`
/// / `plan_edit_discard`, keyed by `edit_id`. `src-tauri::PlanEditProposal`.
export type PlanEditProposal = {
  edit_id: string;
  agent: string;
  path: string;
  original: string;
  proposed: string;
};

// --- core: event.rs ---

/// Token usage an agent reports when a turn completes. `core::Usage`.
export type Usage = {
  input_tokens: number;
  output_tokens: number;
};

/// The normalized event, serialized internally tagged by `kind`
/// (snake_case). `core::NormalizedEvent`. Only `FileChanged` is app-sourced;
/// everything else is adapter-sourced.
export type NormalizedEvent =
  | { kind: "turn_started" }
  | { kind: "assistant_text"; text: string }
  | { kind: "reasoning"; text: string }
  | { kind: "tool_call"; call_id: string; name: string; input_excerpt: string }
  | { kind: "tool_result"; call_id: string; ok: boolean; output_excerpt: string }
  | { kind: "command_run"; cmd: string; exit: number | null }
  | { kind: "turn_completed"; usage: Usage }
  | { kind: "needs_approval" }
  | { kind: "rate_limited"; reset_at: string | null; message: string | null }
  | { kind: "failed"; reason: string }
  | { kind: "ended" }
  | { kind: "file_changed"; path: string };

// --- src-tauri: live event stream payloads (lib.rs) ---

/// A live run event pushed on the `run_event` Tauri event.
/// `src-tauri::RunEventPayload`.
export type RunEventPayload = {
  run_id: string;
  seq: number;
  event: NormalizedEvent;
};

/// A run reaching a terminal state, pushed on the `run_status` Tauri event.
/// `src-tauri::RunStatusPayload`.
export type RunStatusPayload = {
  run_id: string;
  status: RunStatus;
};

/// A rate-limited run's re-run has been scheduled, pushed on the
/// `scheduled_resume` Tauri event. `resume_at` is RFC 3339.
/// `src-tauri::ScheduledResumePayload`.
export type ScheduledResumePayload = {
  run_id: string;
  resume_at: string;
};

/// A previously scheduled re-run was cancelled before it fired, pushed on the
/// `scheduled_resume_cancelled` Tauri event.
/// `src-tauri::ScheduledResumeCancelledPayload`.
export type ScheduledResumeCancelledPayload = {
  run_id: string;
};

/// A launch has been scheduled for later, pushed on the `scheduled_launch`
/// Tauri event. `launch_at` is RFC 3339. `src-tauri::ScheduledLaunchPayload`.
export type ScheduledLaunchPayload = {
  id: number;
  plan_id: string;
  task_anchor: string;
  launch_at: string;
};

/// A scheduled launch fired, pushed on the `scheduled_launch_fired` Tauri
/// event with the run id it produced.
/// `src-tauri::ScheduledLaunchFiredPayload`.
export type ScheduledLaunchFiredPayload = {
  id: number;
  run_id: string;
};

/// A previously scheduled launch was cancelled before it fired, pushed on the
/// `scheduled_launch_cancelled` Tauri event.
/// `src-tauri::ScheduledLaunchCancelledPayload`.
export type ScheduledLaunchCancelledPayload = {
  id: number;
};

/// A scheduled launch was dropped after repeatedly finding the agent still
/// exhausted, pushed on the `scheduled_launch_dropped` Tauri event.
/// `src-tauri::ScheduledLaunchDroppedPayload`.
export type ScheduledLaunchDroppedPayload = {
  id: number;
  plan_id: string;
  task_anchor: string;
  reason: string;
};

/// An agent's limit headroom changed, pushed on the `agent_usage` Tauri event.
/// The payload is the snapshot itself — the agent it describes rides in
/// `agent_key`. Emitted only when the snapshot says something different from
/// what the UI was last told, so `observed_at_ms` alone moving is not an event.
export type AgentUsagePayload = UsageSnapshot;
