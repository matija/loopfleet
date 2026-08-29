import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { finishedRunTone, worktreeBranch } from "./RunDock";
import type { RunStatus } from "../types";

const RUN_DOCK_SOURCE = readFileSync(
  fileURLToPath(new URL("./RunDock.tsx", import.meta.url)),
  "utf8",
);

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

describe("run-chip merged marker", () => {
  it("stays a non-interactive span with its accessible name intact", () => {
    // The merged marker (run-chip__merged) is a status glyph, not a control:
    // no onClick, no button/tabIndex, just role="img" + aria-label so it
    // still reads out to assistive tech. A future edit that turns it into a
    // clickable element or drops the label should fail this test.
    const markerMatch = RUN_DOCK_SOURCE.match(
      /<span\s+className="run-chip__merged"[\s\S]*?<\/span>/,
    );
    expect(markerMatch).not.toBeNull();
    const marker = markerMatch![0];

    expect(marker).toContain('role="img"');
    expect(marker).toContain('aria-label="Merged"');
    expect(marker).not.toMatch(/onClick/);
    expect(marker).not.toMatch(/tabIndex/);
    expect(marker).not.toMatch(/role="button"/);
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
