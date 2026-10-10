// The selected project's plan pane. It hosts two views of the same plan behind a
// segmented toggle: "Tasks" — the parsed checklist with per-task launch controls
// (PlanView) plus the sandbox overrides, which sit below it as a disclosure
// that opens closed (SandboxOverrides) — and "PRD" — the plan file rendered as
// a document (PrdView). The toggle is local to the pane; switching views never
// refetches projects or changes the app-level route.

import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { planOverview } from "../commands";
import { CreatePrd } from "./CreatePrd";
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
  onPlanChanged,
  onOpenAgentSettings,
  toolbarActions,
  prdFocusNonce,
  pendingArchivePlanId,
  onPendingArchiveHandled,
}: {
  projectId: string;
  planNonce: number;
  onPlanChanged: () => void;
  onOpenAgentSettings: () => void;
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
  const tabId = useId();
  const tabs = useRef<Partial<Record<Mode, HTMLButtonElement | null>>>({});
  const [refresh, setRefresh] = useState(0);
  const [availability, setAvailability] = useState<{
    projectId: string;
    planNonce: number;
    refresh: number;
    hasPlan?: boolean;
    error?: string;
  } | null>(null);
  const current = availability?.projectId === projectId && availability.planNonce === planNonce && availability.refresh === refresh ? availability : null;

  useEffect(() => {
    setMode("tasks");
  }, [projectId]);

  useEffect(() => {
    let obsolete = false;
    setAvailability(null);
    planOverview(projectId)
      .then((plans) => {
        if (!obsolete) setAvailability({ projectId, planNonce, refresh, hasPlan: plans.length > 0 });
      })
      .catch((error) => {
        if (!obsolete) setAvailability({ projectId, planNonce, refresh, error: String(error) });
      });
    return () => { obsolete = true; };
  }, [projectId, planNonce, refresh]);

  useEffect(() => {
    if (pendingArchivePlanId) setMode("prd");
  }, [pendingArchivePlanId]);

  useEffect(() => {
    if (prdFocusNonce) setMode("prd");
  }, [prdFocusNonce]);

  const toggle = (
    <div className="plan-toggle" role="tablist" aria-label="Plan views">
      {(["tasks", "prd"] as const).map((tab) => (
        <button
          key={tab}
          ref={(node) => { tabs.current[tab] = node; }}
          type="button"
          role="tab"
          id={`${tabId}-${tab}-tab`}
          aria-controls={`${tabId}-${tab}-panel`}
          aria-selected={mode === tab}
          tabIndex={mode === tab ? 0 : -1}
          className={`plan-toggle__tab${mode === tab ? " plan-toggle__tab--active" : ""}`}
          onClick={() => setMode(tab)}
          onKeyDown={(event) => {
            if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || event.nativeEvent.isComposing) return;
            const destination = event.key === "Home" ? "tasks" : event.key === "End" ? "prd" : event.key === "ArrowLeft" || event.key === "ArrowRight" ? (tab === "tasks" ? "prd" : "tasks") : null;
            if (!destination) return;
            event.preventDefault();
            setMode(destination);
            tabs.current[destination]?.focus();
          }}
        >
          {tab === "tasks" ? "Tasks" : "PRD"}
        </button>
      ))}
    </div>
  );

  return (
    <>
      {current?.hasPlan && (toolbarActions ? createPortal(toggle, toolbarActions) : toggle)}
      <div className="plan-surface">
        {!current ? (
          <p className="plan__loading">Loading plan…</p>
        ) : current.error !== undefined ? (
          <div role="alert">
            <p className="panel__error">{current.error}</p>
            <button type="button" className="btn btn--secondary" onClick={() => setRefresh((value) => value + 1)}>Retry</button>
          </div>
        ) : !current.hasPlan ? (
          <>
            <CreatePrd key={projectId} projectId={projectId} onOpenAgentSettings={onOpenAgentSettings} onCreated={() => {
              setMode("tasks");
              setRefresh((value) => value + 1);
              onPlanChanged();
            }} />
            <SandboxOverrides projectId={projectId} />
          </>
        ) : (
          <>
            <div role="tabpanel" id={`${tabId}-tasks-panel`} aria-labelledby={`${tabId}-tasks-tab`} hidden={mode !== "tasks"}>
              {mode === "tasks" && (
                <>
                  <PlanView
                    key={`${projectId}:${planNonce}`}
                    projectId={projectId}
                    onLaunch={onLaunch}
                    onCompare={onCompare}
                    onError={onError}
                    onOpenPrd={() => { setMode("prd"); tabs.current.prd?.focus(); }}
                  />
                  <SandboxOverrides projectId={projectId} />
                </>
              )}
            </div>
            <div role="tabpanel" id={`${tabId}-prd-panel`} aria-labelledby={`${tabId}-prd-tab`} hidden={mode !== "prd"}>
              {mode === "prd" && (
                <PrdView
                  key={`${projectId}:${planNonce}`}
                  projectId={projectId}
                  onError={onError}
                  onPlanChanged={onPlanChanged}
                  autoArchivePlanId={pendingArchivePlanId}
                  onAutoArchiveHandled={onPendingArchiveHandled}
                />
              )}
            </div>
          </>
        )}
      </div>
    </>
  );
}
