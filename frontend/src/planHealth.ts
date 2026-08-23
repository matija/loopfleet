// Whether a project has anything runnable in it. `plan_overview` answers with
// a plan list, and two of its shapes are dead ends the sidebar should say out
// loud rather than leave as an empty disclosure: a project with no plan file
// at all, and a project whose plan files parse but hold no tasks. Both are
// authoring mistakes (wrong repo registered, a PRD with no `- [ ]` lines), and
// both look identical to a healthy project until you click into it. The
// classification is pure so the sidebar's marker and its explanation stay
// testable without a plan fetch.

import type { PlanView } from "./types";

/// A project's plans as the sidebar cares about them: `no-plan` (nothing
/// matched the plan glob), `no-tasks` (plans exist but every one is taskless),
/// or `ok` (at least one task somewhere).
export type PlanHealth = "no-plan" | "no-tasks" | "ok";

/// Classify a project's loaded plan list. Callers that haven't loaded (or
/// failed to load) a project's plans should not call this — an unknown project
/// gets no marker rather than a wrong one.
export function planHealth(plans: readonly PlanView[]): PlanHealth {
  if (plans.length === 0) return "no-plan";
  return plans.some((plan) => plan.tasks.length > 0) ? "ok" : "no-tasks";
}

/// Whether the health warrants the sidebar's quiet marker. `ok` is the silent
/// majority; the other two both read as "no tasks" at a glance and differ only
/// in the explanation behind them.
export function marksNoTasks(health: PlanHealth): boolean {
  return health !== "ok";
}

/// The marker's `title` — the sentence that tells the two dead ends apart and
/// names the fix. Returns null for `ok`, which shows no marker.
export function planHealthTitle(health: PlanHealth): string | null {
  switch (health) {
    case "no-plan":
      return "No plan file found in this repo — add one to launch runs against it.";
    case "no-tasks":
      return "This project's plan has no tasks — add `- [ ]` lines to launch runs against them.";
    case "ok":
      return null;
  }
}
