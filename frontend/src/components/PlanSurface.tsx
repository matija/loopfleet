// The selected project's plan pane. It hosts two views of the same plan behind a
// segmented toggle: "Tasks" — the parsed checklist with per-task launch controls
// (PlanView) plus the sandbox overrides — and "PRD" — the plan file rendered as a
// document (PrdView). The toggle is local to the pane; switching views never
// refetches projects or changes the app-level route.

import { useState } from "react";
import { PlanView, type CompareTarget, type LaunchedRun } from "./PlanView";
import { PrdView } from "./PrdView";
import { SandboxOverrides } from "./SandboxOverrides";

type Mode = "tasks" | "prd";

export function PlanSurface({
  projectId,
  planNonce,
  onLaunch,
  onCompare,
}: {
  projectId: string;
  planNonce: number;
  onLaunch: (run: LaunchedRun) => void;
  onCompare: (target: CompareTarget) => void;
}) {
  const [mode, setMode] = useState<Mode>("tasks");

  return (
    <>
      <div className="plan-toggle" role="tablist" aria-label="Plan views">
        <button
          role="tab"
          aria-selected={mode === "tasks"}
          className={`plan-toggle__tab${
            mode === "tasks" ? " plan-toggle__tab--active" : ""
          }`}
          onClick={() => setMode("tasks")}
        >
          Tasks
        </button>
        <button
          role="tab"
          aria-selected={mode === "prd"}
          className={`plan-toggle__tab${
            mode === "prd" ? " plan-toggle__tab--active" : ""
          }`}
          onClick={() => setMode("prd")}
        >
          PRD
        </button>
      </div>
      {mode === "tasks" ? (
        <>
          <PlanView
            key={`${projectId}:${planNonce}`}
            projectId={projectId}
            onLaunch={onLaunch}
            onCompare={onCompare}
          />
          <SandboxOverrides projectId={projectId} />
        </>
      ) : (
        <PrdView key={projectId} projectId={projectId} />
      )}
    </>
  );
}
