import { describe, expect, it } from "vitest";
import { crumbsFor } from "./App";
import { taskSummary } from "./displayText";
import { FolderIcon, PlayIcon, DiffIcon } from "./components/Icon";
import type { ActiveRun } from "./components/RunDock";
import type { Project } from "./types";

const project: Project = {
  id: "proj-1",
  repo_path: "/home/dev/repos/loopfleet",
  plan_convention: "prd",
};

const projects: Project[] = [project];

const run: ActiveRun = {
  runId: "run-1",
  projectName: "loopfleet",
  taskText: "Wire up the **run** dock",
  agent: "claude",
  projectId: "proj-1",
  taskAnchor: "anchor-1",
  startedAt: Date.now(),
  status: "running",
};

const runs: ActiveRun[] = [run];

describe("crumbsFor", () => {
  it("overview: a single inert crumb", () => {
    const crumbs = crumbsFor({ kind: "overview" }, projects, runs);
    expect(crumbs).toEqual([{ label: "Overview" }]);
  });

  it("plan: project crumb (clickable) + inert Plan crumb", () => {
    const onSelectProject = () => {};
    const crumbs = crumbsFor(
      { kind: "plan", projectId: "proj-1" },
      projects,
      runs,
      onSelectProject,
    );
    expect(crumbs).toHaveLength(2);
    expect(crumbs[0].label).toBe("loopfleet");
    expect(crumbs[0].icon).toBe(FolderIcon);
    expect(crumbs[1]).toEqual({ label: "Plan" });
  });

  it("plan: falls back to 'Project' label when the project is missing", () => {
    const crumbs = crumbsFor(
      { kind: "plan", projectId: "does-not-exist" },
      projects,
      runs,
    );
    expect(crumbs[0].label).toBe("Project");
    expect(crumbs[1]).toEqual({ label: "Plan" });
  });

  it("plan: project crumb only fires onClick when onSelectProject is passed", () => {
    let selected: string | undefined;
    const crumbs = crumbsFor(
      { kind: "plan", projectId: "proj-1" },
      projects,
      runs,
      (id) => {
        selected = id;
      },
    );
    crumbs[0].onClick?.();
    expect(selected).toBe("proj-1");

    const inert = crumbsFor({ kind: "plan", projectId: "proj-1" }, projects, runs);
    expect(inert[0].onClick).toBeUndefined();
  });

  it("task: project + Plan + normalized task label", () => {
    const crumbs = crumbsFor(
      {
        kind: "task",
        projectId: "proj-1",
        planId: "plan-1",
        taskAnchor: "anchor-1",
        taskText: "Ship the **compare** view",
      },
      projects,
      runs,
    );
    expect(crumbs).toHaveLength(3);
    expect(crumbs[0].label).toBe("loopfleet");
    expect(crumbs[1]).toEqual({ label: "Plan" });
    expect(crumbs[2]).toEqual({ label: "Ship the compare view" });
  });

  it("run: project + Plan + normalized run label, using the run's own project fields", () => {
    const crumbs = crumbsFor({ kind: "run", runId: "run-1" }, projects, runs);
    expect(crumbs).toHaveLength(3);
    expect(crumbs[0].label).toBe("loopfleet");
    expect(crumbs[1]).toEqual({ label: "Plan" });
    expect(crumbs[2]).toEqual({
      label: "Wire up the run dock",
      icon: PlayIcon,
    });
  });

  it("run: falls back to a bare 'Run' crumb when the run is missing", () => {
    const crumbs = crumbsFor(
      { kind: "run", runId: "does-not-exist" },
      projects,
      runs,
    );
    expect(crumbs).toEqual([{ label: "Run", icon: PlayIcon }]);
  });

  it("compare: recovers project/plan from a run sharing the task anchor", () => {
    const crumbs = crumbsFor(
      {
        kind: "compare",
        planId: "plan-1",
        taskAnchor: "anchor-1",
        taskText: "Compare **iterations**",
      },
      projects,
      runs,
    );
    expect(crumbs).toHaveLength(3);
    expect(crumbs[0].label).toBe("loopfleet");
    expect(crumbs[1]).toEqual({ label: "Plan" });
    expect(crumbs[2]).toEqual({
      label: "Compare iterations",
      icon: DiffIcon,
    });
  });

  it("compare: only the task crumb when no run shares the task anchor", () => {
    const crumbs = crumbsFor(
      {
        kind: "compare",
        planId: "plan-1",
        taskAnchor: "no-matching-run",
        taskText: "Compare iterations",
      },
      projects,
      runs,
    );
    expect(crumbs).toEqual([{ label: "Compare iterations", icon: DiffIcon }]);
  });

  it("a long task label is shortened to the compact summary", () => {
    const longText = "Refactor the run dock so it ".repeat(20).trim();
    const crumbs = crumbsFor(
      {
        kind: "task",
        projectId: "proj-1",
        planId: "plan-1",
        taskAnchor: "anchor-1",
        taskText: longText,
      },
      projects,
      runs,
    );
    const taskCrumb = crumbs[crumbs.length - 1];
    expect(taskCrumb.label).toBe(taskSummary(longText));
    expect(taskCrumb.label.length).toBeLessThan(100);
    expect(taskCrumb.label.endsWith("…")).toBe(true);
  });
});
