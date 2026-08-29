import { describe, expect, it } from "vitest";
import { marksNoTasks, planHealth, planHealthTitle } from "./planHealth";
import type { PlanView, TaskView } from "./types";

function task(anchor: string): TaskView {
  return {
    anchor,
    line_hint: 1,
    text: `- [ ] ${anchor}`,
    checked: false,
    status: "not-started",
    run_count: 0,
  };
}

function plan(plan_id: string, tasks: TaskView[]): PlanView {
  return {
    plan_id,
    file_path: `${plan_id}.md`,
    title: plan_id,
    markdown: "",
    tasks,
    unbound_runs: 0,
    drifted_runs: 0,
  };
}

describe("planHealth", () => {
  it("reads an empty plan list as no-plan", () => {
    expect(planHealth([])).toBe("no-plan");
  });

  it("reads plans that all lack tasks as no-tasks", () => {
    expect(planHealth([plan("a", [])])).toBe("no-tasks");
    expect(planHealth([plan("a", []), plan("b", [])])).toBe("no-tasks");
  });

  it("is ok as soon as any plan holds a task", () => {
    expect(planHealth([plan("a", [task("t1")])])).toBe("ok");
    // One taskless plan alongside a populated one is normal, not a dead end.
    expect(planHealth([plan("a", []), plan("b", [task("t1")])])).toBe("ok");
  });
});

describe("marksNoTasks", () => {
  it("marks both dead ends and stays silent on ok", () => {
    expect(marksNoTasks("no-plan")).toBe(true);
    expect(marksNoTasks("no-tasks")).toBe(true);
    expect(marksNoTasks("ok")).toBe(false);
  });
});

describe("planHealthTitle", () => {
  it("explains the two dead ends differently", () => {
    const noPlan = planHealthTitle("no-plan");
    const noTasks = planHealthTitle("no-tasks");
    expect(noPlan).toBeTruthy();
    expect(noTasks).toBeTruthy();
    expect(noPlan).not.toBe(noTasks);
  });

  it("has no title for ok, which shows no marker", () => {
    expect(planHealthTitle("ok")).toBeNull();
  });

  it("gives every marked health an explanation", () => {
    for (const health of ["no-plan", "no-tasks", "ok"] as const) {
      expect(planHealthTitle(health) !== null).toBe(marksNoTasks(health));
    }
  });
});
