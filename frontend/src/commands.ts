// One typed wrapper per Tauri command (PRD M7: "one `commands.ts` typed wrapper
// over `invoke`"). Every command in `src-tauri`'s `generate_handler!` has exactly
// one function here; nothing else calls `invoke` directly. Argument keys are
// camelCase — Tauri v2 maps them to the Rust snake_case parameters.

import { invoke } from "@tauri-apps/api/core";
import type {
  AgentStatus,
  CompareView,
  PlanView,
  Project,
  RunSummary,
  RunTimeline,
  Settings,
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

/// Launch a looping run against a task. Returns the new run id immediately; the
/// loop runs in the background and streams `run_event`/`run_status` events.
export function launchRun(args: {
  projectId: string;
  taskAnchor: string;
  agent: string;
  maxIterations: number;
}): Promise<string> {
  return invoke("launch_run", args);
}

/// Request a stop of an active run (stops at the next pass boundary).
export function stopRun(runId: string): Promise<void> {
  return invoke("stop_run", { runId });
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

/// "Use this run": merge the run's final state into a target branch and mark
/// the run accepted. `targetBranch = null` (or empty) merges into the repo's
/// currently checked-out branch under a descriptive merge commit — the default.
/// A non-empty `targetBranch` names a custom branch (created if absent).
export function useRun(
  runId: string,
  targetBranch: string | null,
): Promise<UseRunResult> {
  return invoke("use_run", { runId, targetBranch: targetBranch ?? null });
}
