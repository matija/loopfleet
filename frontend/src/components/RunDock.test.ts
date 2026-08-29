import { describe, expect, it } from "vitest";
import { finishedRunTone, isMergedRun, worktreeBranch } from "./RunDock";
import type { RunStatus } from "../types";

const ALL_STATUSES: RunStatus[] = [
  "queued",
  "running",
  "completed",
  "failed",
  "stopped",
  "limit-reached",
];

describe("finishedRunTone", () => {
  it("marks a failed run as danger", () => {
    expect(finishedRunTone("failed")).toBe("danger");
  });

  it("marks stopped and rate-limited runs as warn", () => {
    expect(finishedRunTone("stopped")).toBe("warn");
    expect(finishedRunTone("limit-reached")).toBe("warn");
  });

  it("carries no accent for active or successfully-completed runs", () => {
    expect(finishedRunTone("queued")).toBeUndefined();
    expect(finishedRunTone("running")).toBeUndefined();
    expect(finishedRunTone("completed")).toBeUndefined();
  });

  it("covers every status", () => {
    for (const status of ALL_STATUSES) {
      expect(() => finishedRunTone(status)).not.toThrow();
    }
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
});

describe("worktreeBranch", () => {
  it("prefixes the run id with agent/", () => {
    expect(worktreeBranch("abc123")).toBe("agent/abc123");
  });

  it("is deterministic for a given run id", () => {
    expect(worktreeBranch("xyz")).toBe(worktreeBranch("xyz"));
  });
});
