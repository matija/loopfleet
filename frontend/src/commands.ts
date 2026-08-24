// One typed wrapper per Tauri command (PRD M7: "one `commands.ts` typed wrapper
// over `invoke`"). Every command in `src-tauri`'s `generate_handler!` has exactly
// one function here; nothing else calls `invoke` directly. Argument keys are
// camelCase — Tauri v2 maps them to the Rust snake_case parameters.

import { invoke } from "@tauri-apps/api/core";
import type {
  AgentStatus,
  CompareView,
  PlanEditProposal,
  PlanView,
  Project,
  RunSummary,
  RunTimeline,
  Settings,
  SweepResult,
  UseRunResult,
} from "./types";

/// Validate `path` is a git repo and persist it as a project.
export function registerProject(path: string): Promise<Project> {
  return invoke("register_project", { path });
}

/// All registered projects.
export function listProjects(): Promise<Project[]> {
  return invoke("list_projects");
}

/// Discover the v1 agent CLIs: availability, version, drift.
export function agentStatus(): Promise<AgentStatus[]> {
  return invoke("agent_status");
}

/// The global app settings.
export function getSettings(): Promise<Settings> {
  return invoke("get_settings");
}

/// Persist the global app settings.
export function saveSettings(settings: Settings): Promise<void> {
  return invoke("save_settings", { settings });
}

/// A project's sandbox write overrides (extra absolute paths granted per run).
export function projectSandboxWrites(projectId: string): Promise<string[]> {
  return invoke("project_sandbox_writes", { projectId });
}

/// Replace a project's sandbox write overrides. Each path must be absolute.
export function setProjectSandboxWrites(
  projectId: string,
  paths: string[],
): Promise<void> {
  return invoke("set_project_sandbox_writes", { projectId, paths });
}

/// The plan overview for a project (derived `TaskStatus` per task).
export function planOverview(projectId: string): Promise<PlanView[]> {
  return invoke("plan_overview", { projectId });
}

/// The raw markdown of a single plan document, by plan id. Read-only: no store
/// sync, for rendering the full frozen PRD on demand.
export function planDocument(planId: string): Promise<string> {
  return invoke("plan_document", { planId });
}

/// Run one AI pass over a plan document and return the proposed edit for review.
/// The default agent edits the PRD in an isolated worktree against `instruction`;
/// nothing is written to the real file until `planEditApply`. Rejects with an
/// explicit message when no default agent is installed or the pass fails.
export function planEdit(
  planId: string,
  instruction: string,
): Promise<PlanEditProposal> {
  return invoke("plan_edit", { planId, instruction });
}

/// Accept a proposed AI plan edit (`edit_id`): write the proposed markdown to the
/// real PRD file and drop the scratch worktree.
export function planEditApply(editId: string): Promise<void> {
  return invoke("plan_edit_apply", { editId });
}

/// Discard a proposed AI plan edit (`edit_id`): drop the scratch worktree,
/// writing nothing.
export function planEditDiscard(editId: string): Promise<void> {
  return invoke("plan_edit_discard", { editId });
}

/// Launch a looping run against a task. Returns the new run id immediately; the
/// loop runs in the background and streams `run_event`/`run_status` events.
export function launchRun(args: {
  projectId: string;
  taskAnchor: string;
  agent: string;
  /// Model override for this run (e.g. Claude's "opus"/"sonnet", or a pinned
  /// version string). `undefined`/omitted uses the agent CLI's own default.
  model?: string | null;
  maxIterations: number;
}): Promise<string> {
  return invoke("launch_run", { ...args, model: args.model ?? null });
}

/// Request a stop of an active run (stops at the next pass boundary).
export function stopRun(runId: string): Promise<void> {
  return invoke("stop_run", { runId });
}

/// Abort a pending rate-limit re-run before it fires, keyed by the original
/// run's id.
export function cancelScheduledResume(runId: string): Promise<void> {
  return invoke("cancel_scheduled_resume", { runId });
}

/// Run a worktree sweep pass immediately (same eligibility rules as the
/// hourly background sweep) and report how much it reclaimed, for the
/// settings panel's "Clean up now" control.
export function sweepWorktreesNow(): Promise<SweepResult> {
  return invoke("sweep_worktrees_now");
}

/// Clear the OS-level dock badge/attention signal — the command counterpart to
/// clearing the in-app `unseen` marker on focus or per-run acknowledge.
export function acknowledgeRuns(): Promise<void> {
  return invoke("acknowledge_runs");
}

/// Every run bound to any task in a plan.
export function planRuns(planId: string): Promise<RunSummary[]> {
  return invoke("plan_runs", { planId });
}

/// A run's timeline: iterations, per-iteration events, per-iteration diff.
export function runTimeline(runId: string): Promise<RunTimeline> {
  return invoke("run_timeline", { runId });
}

/// The compare view for a task: every run side by side with its final diff.
export function compareTask(
  planId: string,
  taskAnchor: string,
): Promise<CompareView> {
  return invoke("compare_task", { planId, taskAnchor });
}

/// Export one task's stored data as a standalone HTML report via a native save
/// dialog (defaulting to a name built from the task's text), revealed in
/// Finder on success. Resolves to the saved path, or `null` if cancelled.
export function exportTaskReport(
  planId: string,
  taskAnchor: string,
): Promise<string | null> {
  return invoke("export_task_report", { planId, taskAnchor });
}

/// Export a whole plan's stored data as a standalone HTML report via a native
/// save dialog (defaulting to a name built from the plan's title), revealed in
/// Finder on success. Resolves to the saved path, or `null` if cancelled.
export function exportPlanReport(planId: string): Promise<string | null> {
  return invoke("export_plan_report", { planId });
}

/// "Use this run": merge the run's final state into a target branch and mark
/// the run accepted. `targetBranch = null` (or empty) merges into the repo's
/// currently checked-out branch as one squashed commit with a descriptive
/// message — the default.
/// A non-empty `targetBranch` names a custom branch (created if absent).
export function useRun(
  runId: string,
  targetBranch: string | null,
): Promise<UseRunResult> {
  return invoke("use_run", { runId, targetBranch: targetBranch ?? null });
}
