// Shared run-status vocabulary: the single source for how a `RunStatus` reads in
// the UI and whether it still counts as active. Every run surface (the dock, the
// live view, the timeline, compare) imports these so the label text, leading
// glyph, and the active/finished split never drift between them. Pairs with
// `.status-pill` in status.css, which owns the matching per-status colors.

import type { JSX } from "react";
import type { RunStatus } from "./types";
import {
  AlertIcon,
  CheckIcon,
  ClockIcon,
  DotIcon,
  PlayIcon,
  SquareIcon,
  type IconProps,
} from "./components/Icon";

/// Human labels for each lifecycle token. Matches `RunStatus` exactly.
export const RUN_STATUS_LABEL: Record<RunStatus, string> = {
  queued: "Queued",
  running: "Running",
  completed: "Completed",
  failed: "Failed",
  stopped: "Stopped",
  "limit-reached": "Rate-limited",
};

/// Meta line for a rate-limited run whose automatic resume chain hit the
/// backend's attempt cap (`MAX_RESUME_ATTEMPTS`): no further re-run is coming,
/// the run is left to the user. One shared string so every surface reads the
/// same "no more retries coming" wording beside the `limit-reached` label.
export const RETRIES_EXHAUSTED_LABEL = "Rate-limited · retries exhausted";

/// Leading glyph per lifecycle token, drawn in currentColor so it always
/// matches the status text it sits beside.
export const RUN_STATUS_ICON: Record<RunStatus, (props: IconProps) => JSX.Element> = {
  queued: DotIcon,
  running: PlayIcon,
  completed: CheckIcon,
  failed: AlertIcon,
  stopped: SquareIcon,
  "limit-reached": ClockIcon,
};

/// A run still doing work — stoppable, with no final diff to apply yet.
export function isActiveRun(status: RunStatus): boolean {
  return status === "queued" || status === "running";
}

/// The subset of a dock entry that decides whether "use this run" applies.
/// Structural on purpose: `RunDock`'s `ActiveRun` satisfies it, and so does any
/// other surface's run shape, without status.ts depending on a component.
export type MergeCandidate = {
  status: RunStatus;
  /// `undefined` until the run's detail has loaded — treated as "not known yet",
  /// which is not a green light.
  accepted?: boolean;
  mergeable?: boolean;
  pendingResume?: { resumeAt: number };
};

/// Whether the dock may offer a merge for this run: the run is finished, ended
/// in `completed` (a failed/stopped/rate-limited run has nothing we'd offer to
/// land), hasn't already been accepted, produced a snapshot to merge, and has no
/// automatic re-run pending — a scheduled resume means the run isn't done
/// changing, so merging it now would land a half-finished attempt.
export function canMergeFromDock(run: MergeCandidate): boolean {
  return (
    !isActiveRun(run.status) &&
    run.status === "completed" &&
    run.accepted !== true &&
    run.mergeable === true &&
    run.pendingResume === undefined
  );
}

/// Whether a run has already landed ("use this run" merged it): finished, and
/// its detail says it was accepted. Every merge-control surface (the dock chip,
/// the run timeline, compare) swaps its action for a static merged marker under
/// this condition rather than leaving a live merge control on a run that's
/// already landed.
export function isMergedRun(run: {
  status: RunStatus;
  accepted?: boolean;
}): boolean {
  return !isActiveRun(run.status) && run.accepted === true;
}
