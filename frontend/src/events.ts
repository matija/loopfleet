// Typed wrappers over the live Tauri event streams the backend pushes (PRD M7:
// "one `events.ts` for the `run_event`/`run_status` streams", since joined by
// the scheduled-resume and per-agent usage streams). Each
// returns the `UnlistenFn` promise from `@tauri-apps/api/event` — await it and
// call the result to stop listening (e.g. in a React effect cleanup).

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AgentUsagePayload,
  AutoAdvanceStoppedPayload,
  AutoMergeArmedPayload,
  AutoMergeCancelledPayload,
  AutoMergeFailedPayload,
  AutoMergePendingQuestionPayload,
  AutopilotResumePromptPayload,
  RunEventPayload,
  RunStatusPayload,
  ScheduledLaunchCancelledPayload,
  ScheduledLaunchDroppedPayload,
  ScheduledLaunchFiredPayload,
  ScheduledLaunchPayload,
  ScheduledResumeCancelledPayload,
  ScheduledResumePayload,
} from "./types";

/// Subscribe to per-event updates for any run. The callback receives the run id,
/// the event's `seq`, and the normalized event payload.
export function onRunEvent(
  handler: (payload: RunEventPayload) => void,
): Promise<UnlistenFn> {
  return listen<RunEventPayload>("run_event", (e) => handler(e.payload));
}

/// Subscribe to run terminal-state updates. Fires once per run when it reaches
/// `completed` / `failed` / `stopped`.
export function onRunStatus(
  handler: (payload: RunStatusPayload) => void,
): Promise<UnlistenFn> {
  return listen<RunStatusPayload>("run_status", (e) => handler(e.payload));
}

/// Subscribe to a rate-limited run's re-run being scheduled — the resume time
/// rides `resume_at` (RFC 3339).
export function onScheduledResume(
  handler: (payload: ScheduledResumePayload) => void,
): Promise<UnlistenFn> {
  return listen<ScheduledResumePayload>("scheduled_resume", (e) =>
    handler(e.payload),
  );
}

/// Subscribe to a scheduled re-run being cancelled before it fired.
export function onScheduledResumeCancelled(
  handler: (payload: ScheduledResumeCancelledPayload) => void,
): Promise<UnlistenFn> {
  return listen<ScheduledResumeCancelledPayload>(
    "scheduled_resume_cancelled",
    (e) => handler(e.payload),
  );
}

/// Subscribe to a launch being scheduled for later — the launch time rides
/// `launch_at` (RFC 3339).
export function onScheduledLaunch(
  handler: (payload: ScheduledLaunchPayload) => void,
): Promise<UnlistenFn> {
  return listen<ScheduledLaunchPayload>("scheduled_launch", (e) =>
    handler(e.payload),
  );
}

/// Subscribe to a scheduled launch firing — the payload carries the run id it
/// produced.
export function onScheduledLaunchFired(
  handler: (payload: ScheduledLaunchFiredPayload) => void,
): Promise<UnlistenFn> {
  return listen<ScheduledLaunchFiredPayload>("scheduled_launch_fired", (e) =>
    handler(e.payload),
  );
}

/// Subscribe to a scheduled launch being cancelled before it fired.
export function onScheduledLaunchCancelled(
  handler: (payload: ScheduledLaunchCancelledPayload) => void,
): Promise<UnlistenFn> {
  return listen<ScheduledLaunchCancelledPayload>(
    "scheduled_launch_cancelled",
    (e) => handler(e.payload),
  );
}

/// Subscribe to a scheduled launch being dropped after the agent was found
/// still exhausted through `MAX_LAUNCH_RESCHEDULES` pre-fire re-checks —
/// `reason` is a user-facing sentence explaining why.
export function onScheduledLaunchDropped(
  handler: (payload: ScheduledLaunchDroppedPayload) => void,
): Promise<UnlistenFn> {
  return listen<ScheduledLaunchDroppedPayload>(
    "scheduled_launch_dropped",
    (e) => handler(e.payload),
  );
}

/// Subscribe to per-agent limit headroom changes. Fires when an agent's
/// snapshot starts saying something different — a refreshed `agentUsage` probe,
/// or a rate limit observed mid-run — so a usage meter can stay live without
/// polling. Not a heartbeat: an unchanged snapshot is not re-emitted.
export function onAgentUsage(
  handler: (payload: AgentUsagePayload) => void,
): Promise<UnlistenFn> {
  return listen<AgentUsagePayload>("agent_usage", (e) => handler(e.payload));
}

/// Subscribe to the startup question asking whether to resume a stalled
/// `auto_advance` chain — fires once per restart, for the oldest recovered
/// scheduled launch, and only when auto-advance left one un-armed.
export function onAutopilotResumePrompt(
  handler: (payload: AutopilotResumePromptPayload) => void,
): Promise<UnlistenFn> {
  return listen<AutopilotResumePromptPayload>(
    "autopilot_resume_prompt",
    (e) => handler(e.payload),
  );
}

/// Subscribe to the startup question asking whether to merge a run that was
/// completed, unaccepted, and mergeable when the app last quit — its
/// in-memory auto-merge countdown didn't survive the restart.
export function onAutoMergePendingQuestion(
  handler: (payload: AutoMergePendingQuestionPayload) => void,
): Promise<UnlistenFn> {
  return listen<AutoMergePendingQuestionPayload>(
    "auto_merge_pending_question",
    (e) => handler(e.payload),
  );
}

/// Subscribe to a run's auto-merge countdown arming — the merge time rides
/// `merge_at` (RFC 3339).
export function onAutoMergeArmed(
  handler: (payload: AutoMergeArmedPayload) => void,
): Promise<UnlistenFn> {
  return listen<AutoMergeArmedPayload>("auto_merge_armed", (e) =>
    handler(e.payload),
  );
}

/// Subscribe to a previously armed auto-merge countdown being cancelled
/// before it fired.
export function onAutoMergeCancelled(
  handler: (payload: AutoMergeCancelledPayload) => void,
): Promise<UnlistenFn> {
  return listen<AutoMergeCancelledPayload>("auto_merge_cancelled", (e) =>
    handler(e.payload),
  );
}

/// Subscribe to a fired auto-merge attempt failing — a dirty-tree refusal, a
/// conflict, or any other error from the merge itself.
export function onAutoMergeFailed(
  handler: (payload: AutoMergeFailedPayload) => void,
): Promise<UnlistenFn> {
  return listen<AutoMergeFailedPayload>("auto_merge_failed", (e) =>
    handler(e.payload),
  );
}

/// Subscribe to auto-advance declining to chain the plan's next task after an
/// auto-merge — the concurrency cap was met, a launch was already pending, or
/// the plan had no not-started task left.
export function onAutoAdvanceStopped(
  handler: (payload: AutoAdvanceStoppedPayload) => void,
): Promise<UnlistenFn> {
  return listen<AutoAdvanceStoppedPayload>("auto_advance_stopped", (e) =>
    handler(e.payload),
  );
}

/// Subscribe to the app menu's "About Loopfleet" item. The menu lives on the
/// Rust side; what it opens is the frontend's business, so the item only
/// announces itself and this side shows the panel.
export function onMenuAbout(handler: () => void): Promise<UnlistenFn> {
  return listen("menu_about", () => handler());
}

/// Subscribe to the app menu's "Check for Updates…" item. Same split as
/// [`onMenuAbout`]: the menu announces, the frontend runs the check it already
/// owns at launch.
export function onMenuCheckUpdates(handler: () => void): Promise<UnlistenFn> {
  return listen("menu_check_updates", () => handler());
}
