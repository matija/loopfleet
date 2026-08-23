import { describe, expect, it } from "vitest";
import { canMergeFromDock, isActiveRun, type MergeCandidate } from "./status";
import type { RunStatus } from "./types";

const ALL_STATUSES: RunStatus[] = [
  "queued",
  "running",
  "completed",
  "failed",
  "stopped",
  "limit-reached",
];

/// A run that satisfies every clause — each test negates exactly one.
function mergeable(over: Partial<MergeCandidate> = {}): MergeCandidate {
  return { status: "completed", accepted: false, mergeable: true, ...over };
}

describe("isActiveRun", () => {
  it("counts only the pre-terminal tokens as active", () => {
    expect(ALL_STATUSES.filter(isActiveRun)).toEqual(["queued", "running"]);
  });
});

describe("canMergeFromDock", () => {
  it("is true for a terminal, completed, unaccepted, mergeable run", () => {
    expect(canMergeFromDock(mergeable())).toBe(true);
  });

  it("is true for exactly one status", () => {
    const yes = ALL_STATUSES.filter((status) =>
      canMergeFromDock(mergeable({ status })),
    );
    expect(yes).toEqual(["completed"]);
  });

  it("is false for a run already accepted", () => {
    expect(canMergeFromDock(mergeable({ accepted: true }))).toBe(false);
  });

  it("is false with nothing to merge", () => {
    expect(canMergeFromDock(mergeable({ mergeable: false }))).toBe(false);
  });

  it("is false while a re-run is still pending", () => {
    expect(
      canMergeFromDock(mergeable({ pendingResume: { resumeAt: 1 } })),
    ).toBe(false);
    // Even a resume scheduled in the past blocks it — App.tsx clears the field
    // when the resume actually fires.
    expect(
      canMergeFromDock(mergeable({ pendingResume: { resumeAt: 0 } })),
    ).toBe(false);
  });

  it("treats unloaded detail as not-yet-mergeable rather than mergeable", () => {
    // `undefined` means the run's detail hasn't loaded; don't offer a merge on a
    // guess.
    expect(canMergeFromDock({ status: "completed" })).toBe(false);
    expect(canMergeFromDock(mergeable({ mergeable: undefined }))).toBe(false);
    expect(canMergeFromDock(mergeable({ accepted: undefined }))).toBe(true);
  });

  it("does not mutate the run it inspects", () => {
    const run = mergeable();
    const before = structuredClone(run);
    canMergeFromDock(run);
    expect(run).toEqual(before);
  });
});
