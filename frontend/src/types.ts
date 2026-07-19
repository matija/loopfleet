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
  merged_commit: string;
  created: boolean;
  up_to_date: boolean;
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
