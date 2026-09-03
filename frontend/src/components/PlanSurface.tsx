// The selected project's plan pane. It hosts two views of the same plan behind a
// segmented toggle: "Tasks" — the parsed checklist with per-task launch controls
// (PlanView) plus the sandbox overrides, which sit below it as a disclosure
// that opens closed (SandboxOverrides) — and "PRD" — the plan file rendered as
// a document (PrdView). The toggle is local to the pane; switching views never
// refetches projects or changes the app-level route.

import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { PlanView, type CompareTarget, type LaunchedRun } from "./PlanView";
import { PrdView } from "./PrdView";
import { SandboxOverrides } from "./SandboxOverrides";

type Mode = "tasks" | "prd";

export function PlanSurface({
  projectId,
  planNonce,
  onLaunch,
  onCompare,
  onError,
  toolbarActions,
  prdFocusNonce,
  pendingArchivePlanId,
  onPendingArchiveHandled,
}: {
  projectId: string;
  planNonce: number;
  onLaunch: (run: LaunchedRun) => void;
  onCompare: (target: CompareTarget) => void;
  /// Surfaces command failures (a failed export) through the app's toasts.
  onError: (message: string) => void;
  /// The toolbar's action-slot DOM node. The Tasks/PRD toggle portals there
  /// instead of rendering inline.
  toolbarActions: HTMLElement | null;
  /// Bumped by the sidebar's plan-title click to force the PRD view open for
  /// the plan just clicked. A counter rather than a flag so clicking the same
  /// title again after switching back to Tasks still opens the document.
  prdFocusNonce?: number;
  /// A plan id to jump straight into the archive confirmation for, set by
  /// the command palette's "Archive plan" action. Forces the PRD view open
  /// so PrdView can consume it.
  pendingArchivePlanId?: string | null;
  onPendingArchiveHandled?: () => void;
}) {
  const [mode, setMode] = useState<Mode>("tasks");

  useEffect(() => {
    if (pendingArchivePlanId) setMode("prd");
  }, [pendingArchivePlanId]);

  useEffect(() => {
    if (prdFocusNonce) setMode("prd");
  }, [prdFocusNonce]);

  const toggle = (
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
  );

  return (
    <>
      {toolbarActions ? createPortal(toggle, toolbarActions) : toggle}
      {mode === "tasks" ? (
        <>
          <PlanView
            key={`${projectId}:${planNonce}`}
            projectId={projectId}
            onLaunch={onLaunch}
            onCompare={onCompare}
            onError={onError}
          />
          <SandboxOverrides projectId={projectId} />
        </>
      ) : (
        <PrdView
          key={projectId}
          projectId={projectId}
          onError={onError}
          autoArchivePlanId={pendingArchivePlanId}
          onAutoArchiveHandled={onPendingArchiveHandled}
        />
      )}
    </>
  );
}
