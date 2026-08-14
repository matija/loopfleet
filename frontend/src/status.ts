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
