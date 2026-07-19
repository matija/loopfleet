// Shared run-status vocabulary: the single source for how a `RunStatus` reads in
// the UI and whether it still counts as active. Every run surface (the dock, the
// live view, the timeline, compare) imports these so the label text and the
// active/finished split never drift between them. Pairs with `.status-pill` in
// status.css, which owns the matching per-status colors.

import type { RunStatus } from "./types";

/// Human labels for each lifecycle token. Matches `RunStatus` exactly.
export const RUN_STATUS_LABEL: Record<RunStatus, string> = {
  queued: "Queued",
  running: "Running",
  completed: "Completed",
  failed: "Failed",
  stopped: "Stopped",
  "limit-reached": "Rate-limited",
};

/// A run still doing work — stoppable, with no final diff to apply yet.
export function isActiveRun(status: RunStatus): boolean {
  return status === "queued" || status === "running";
}
