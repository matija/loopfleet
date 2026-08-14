// Typed wrappers over the two live Tauri event streams the run loop emits
// (PRD M7: "one `events.ts` for the `run_event`/`run_status` streams"). Each
// returns the `UnlistenFn` promise from `@tauri-apps/api/event` — await it and
// call the result to stop listening (e.g. in a React effect cleanup).

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  RunEventPayload,
  RunStatusPayload,
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
