import { describe, expect, it } from "vitest";
import {
  canMergeFromDock,
  isActiveRun,
  isMergedRun,
  type MergeCandidate,
} from "./status";
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

describe("isMergedRun", () => {
  it("is true only for a finished, accepted run", () => {
    expect(isMergedRun({ status: "completed", accepted: true })).toBe(true);
  });

  it("is false while the run is still active, even if marked accepted", () => {
    expect(isMergedRun({ status: "queued", accepted: true })).toBe(false);
    expect(isMergedRun({ status: "running", accepted: true })).toBe(false);
  });

  it("is false for a finished run that wasn't accepted", () => {
    expect(isMergedRun({ status: "completed", accepted: false })).toBe(false);
    expect(isMergedRun({ status: "completed" })).toBe(false);
  });

  it("is false for a finished, unaccepted run regardless of terminal status", () => {
    for (const status of ["failed", "stopped", "limit-reached"] as const) {
      expect(isMergedRun({ status, accepted: false })).toBe(false);
    }
  });

  it("leaves both the merged marker and the merge action off a completed run with nothing to land", () => {
    // A completed run that produced no mergeable diff: not merged, and the
    // dock has no merge to offer. This must be legible as "nothing to do"
    // rather than a state neither slot accounts for — same rule the dock chip,
    // the run timeline, and compare all share via this one helper.
    const run = { status: "completed" as const, accepted: false, mergeable: false };
    expect(isMergedRun(run)).toBe(false);
    expect(canMergeFromDock(run)).toBe(false);
  });
});
