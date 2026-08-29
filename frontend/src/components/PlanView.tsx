// Plan view: the frozen PRD's task list with a derived `TaskStatus` overlay and
// a launch control on EVERY task (PRD M7). The launch control is deliberately
// decoupled from the authored `checked` flag — `checked` only gates the derived
// status, never the ability to start a run — so a "done" plan (every box checked)
// still shows a Run button per task. Completed-unaccepted tasks are summarized
// in one quiet banner above the list; each affected row carries the amber
// status glyph as its own signal (the compare/accept backlog).

import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useAgentUsage } from "../agentUsage";
import {
  agentStatus,
  checkAgentUsage,
  continuePlan,
  exportPlanReport,
  getSettings,
  launchRun,
  listProjects,
  planOverview,
  scheduleLaunch,
} from "../commands";
import { taskSummary } from "../displayText";
import { onRunStatus } from "../events";
import { preferredAgent, readLaunchPrefs, writeLaunchPrefs } from "../launchPrefs";
import { isActiveRun, RUN_STATUS_LABEL } from "../status";
import {
  formatCountdown,
  formatResetTime,
  launchHeadroom,
  usageIndicator,
} from "../usage";
import { SplitButton } from "./Button";
import { formatDuration } from "./DataGrid";
import { Elapsed } from "./Elapsed";
import { NoPlanEmptyState, NoTasksEmptyState } from "./EmptyState";
import { ExportButton } from "./ExportButton";
import {
  AgentIcon,
  AlertIcon,
  BoxIcon,
  CheckIcon,
  ClockIcon,
  DotIcon,
  FolderIcon,
  GitBranchIcon,
  PlayIcon,
} from "./Icon";
import { Popover } from "./Popover";
import { finishedRunTone, MetaRow, useHoverOpen, worktreeBranch } from "./RunDock";
import type {
  AgentStatus,
  AgentUsageCheck,
  PlanView as Plan,
  RunStatus,
  TaskStatus,
  TaskView,
} from "../types";

export const STATUS_LABEL: Record<TaskStatus, string> = {
  "not-started": "Not started",
  "in-progress": "In progress",
  "completed-unaccepted": "Needs review",
  accepted: "Accepted",
};

export const STATUS_ICON: Record<TaskStatus, typeof CheckIcon> = {
  "not-started": DotIcon,
  "in-progress": ClockIcon,
  "completed-unaccepted": AlertIcon,
  accepted: CheckIcon,
};

/// What a task launch reports upward for the global run dock.
export type LaunchedRun = {
  runId: string;
  taskText: string;
  taskAnchor: string;
  agent: string;
  model: string;
  maxIterations: number;
};

/// Model presets offered in the launch menu, per agent — only agents whose
/// CLI accepts `--model` (Claude Code, pi) get an entry. Free text is still
/// accepted for anything not listed (e.g. a pinned version like
/// "claude-opus-4-1-20250805", or a pi "provider/id" pattern) — these are
/// just the common shortcuts.
const AGENT_MODEL_PRESETS: Record<string, string[]> = {
  claude: ["opus", "sonnet", "haiku"],
  pi: ["opus", "sonnet", "haiku"],
};

/// What opening the compare view needs: the plan + task and its display text.
export type CompareTarget = {
  planId: string;
  taskAnchor: string;
  taskText: string;
};

export function PlanView({
  projectId,
  onLaunch,
  onCompare,
  onError,
}: {
  projectId: string;
  onLaunch: (run: LaunchedRun) => void;
  onCompare: (target: CompareTarget) => void;
  /// Surfaces command failures (a failed export) through the app's toasts.
  onError: (message: string) => void;
}) {
  const [plans, setPlans] = useState<Plan[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [agents, setAgents] = useState<AgentStatus[]>([]);
  const [agentsLoading, setAgentsLoading] = useState(true);
  const [repoName, setRepoName] = useState<string | undefined>(undefined);

  const reload = useCallback(() => {
    planOverview(projectId)
      .then(setPlans)
      .catch((e) => setError(String(e)));
  }, [projectId]);

  // Reset and reload whenever the selected project changes.
  useEffect(() => {
    setPlans(null);
    setError(null);
    reload();
  }, [reload]);

  // The agent menu. Small, stable — fetched once.
  useEffect(() => {
    agentStatus()
      .then(setAgents)
      .catch(() => {})
      .finally(() => setAgentsLoading(false));
  }, []);

  // The project's repo name, for the task rows' metadata hover card — plan
  // overview data doesn't carry it, so it's resolved separately.
  useEffect(() => {
    listProjects()
      .then((ps) => {
        const p = ps.find((x) => x.id === projectId);
        if (p) {
          const parts = p.repo_path.replace(/\/+$/, "").split("/");
          setRepoName(parts[parts.length - 1] || p.repo_path);
        }
      })
      .catch(() => {});
  }, [projectId]);

  const installed = agents.filter((a) => a.installed).map((a) => a.key);

  if (error) return <p className="panel__error">{error}</p>;
  if (!plans) return <p className="plan__loading">Loading plan…</p>;
  if (plans.length === 0) return <NoPlanEmptyState />;

  return (
    <div className="plans">
      {plans.map((plan) => (
        <PlanCard
          key={plan.plan_id}
          plan={plan}
          projectId={projectId}
          repoName={repoName}
          installed={installed}
          agentsLoading={agentsLoading}
          onLaunched={reload}
          onLaunch={onLaunch}
          onCompare={onCompare}
          onError={onError}
        />
      ))}
    </div>
  );
}

function PlanCard({
  plan,
  projectId,
  repoName,
  installed,
  agentsLoading,
  onLaunched,
  onLaunch,
  onCompare,
  onError,
}: {
  plan: Plan;
  projectId: string;
  repoName: string | undefined;
  installed: string[];
  agentsLoading: boolean;
  onLaunched: () => void;
  onLaunch: (run: LaunchedRun) => void;
  onCompare: (target: CompareTarget) => void;
  onError: (message: string) => void;
}) {
  const review = plan.tasks.filter((t) => t.status === "completed-unaccepted");
  // Accepted tasks fall out of the working set: they render in their own
  // "Done" section below the open tasks (authored order preserved within each
  // group), so the index leads with what's left to do.
  const open = plan.tasks.filter((t) => t.status !== "accepted");
  const done = plan.tasks.filter((t) => t.status === "accepted");
  // Same rule the backend's `next_task` uses to pick what "Continue plan"
  // would start — mirrored here just to decide whether to show the button.
  const hasNextTask = plan.tasks.some((t) => t.status === "not-started");

  return (
    <section className="plan-card">
      <header className="plan-card__head">
        <h3>{plan.title ?? plan.file_path}</h3>
        <span className="plan-card__path">{plan.file_path}</span>
        {hasNextTask && (
          <ContinuePlanButton
            projectId={projectId}
            planId={plan.plan_id}
            onLaunched={onLaunched}
            onLaunch={onLaunch}
            onError={onError}
          />
        )}
        <ExportButton
          onExport={() => exportPlanReport(plan.plan_id)}
          onError={onError}
          title="Save every task in this plan — its runs, events, and diffs — as an HTML report"
        />
      </header>

      {review.length > 0 && (
        <div className="review-banner" role="status">
          <AlertIcon size={16} className="review-banner__icon" />
          <span>
            <strong>{review.length}</strong>{" "}
            {review.length === 1 ? "run is" : "runs are"} awaiting review —
            compare the produced diffs and use one, or keep iterating.
          </span>
        </div>
      )}

      {plan.tasks.length === 0 ? (
        <NoTasksEmptyState />
      ) : (
        <>
          {open.length > 0 && (
            <ul className="task-list">
              {open.map((task) => (
                <TaskRow
                  key={task.anchor}
                  task={task}
                  planId={plan.plan_id}
                  projectId={projectId}
                  repoName={repoName}
                  installed={installed}
                  agentsLoading={agentsLoading}
                  onLaunched={onLaunched}
                  onLaunch={onLaunch}
                  onCompare={onCompare}
                />
              ))}
            </ul>
          )}
          {done.length > 0 && (
            <>
              <div className="task-list__done-head">
                <CheckIcon size={14} className="task-list__done-icon" />
                <span>
                  Done · {done.length} accepted
                  {open.length === 0 ? " — all tasks accepted" : ""}
                </span>
              </div>
              <ul className="task-list task-list--done">
                {done.map((task) => (
                  <TaskRow
                    key={task.anchor}
                    task={task}
                    planId={plan.plan_id}
                    projectId={projectId}
                    repoName={repoName}
                    installed={installed}
                    agentsLoading={agentsLoading}
                    onLaunched={onLaunched}
                    onLaunch={onLaunch}
                    onCompare={onCompare}
                  />
                ))}
              </ul>
            </>
          )}
        </>
      )}
    </section>
  );
}

/// Starts the plan's next not-started task with the plan's last-used launch
/// preferences (or the app defaults, if it's never had a run) — one click to
/// keep a chain moving without opening a per-task launch menu. Reports the
/// launched run upward through the same `onLaunch`/`onLaunched` callbacks a
/// per-task `LaunchControl` uses, so it folds into the global run dock the
/// same way.
function ContinuePlanButton({
  projectId,
  planId,
  onLaunched,
  onLaunch,
  onError,
}: {
  projectId: string;
  planId: string;
  onLaunched: () => void;
  onLaunch: (run: LaunchedRun) => void;
  onError: (message: string) => void;
}) {
  const [busy, setBusy] = useState(false);

  async function run() {
    if (busy) return;
    setBusy(true);
    try {
      const result = await continuePlan({ projectId, planId });
      onLaunch({
        runId: result.run_id,
        taskText: result.task_text,
        taskAnchor: result.task_anchor,
        agent: result.agent,
        model: result.model ?? "",
        maxIterations: result.max_iterations,
      });
      onLaunched();
    } catch (e) {
      onError(`Continue plan failed: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <button
      className="btn btn--quiet"
      onClick={run}
      disabled={busy}
      title="Start the plan's next not-started task, using this plan's last-used agent, model, and pass count"
    >
      <PlayIcon size={14} className="btn__icon" />
      {busy ? "Starting…" : "Continue plan"}
    </button>
  );
}

/// What the row knows about the most recent run it launched this session —
/// enough to populate the metadata hover card. Not persisted; a fresh mount
/// (e.g. reopening the plan) starts with none until the row launches again.
type RowLastRun = {
  runId: string;
  agent: string;
  model: string;
  maxIterations: number;
  startedAt: number;
  status: RunStatus;
  /// Set once the run leaves an active status — there's no server "finished
  /// at" field, so this is the moment the row itself observed the change.
  finishedAt?: number;
};

function TaskRow({
  task,
  planId,
  projectId,
  repoName,
  installed,
  agentsLoading,
  onLaunched,
  onLaunch,
  onCompare,
}: {
  task: TaskView;
  planId: string;
  projectId: string;
  repoName: string | undefined;
  installed: string[];
  agentsLoading: boolean;
  onLaunched: () => void;
  onLaunch: (run: LaunchedRun) => void;
  onCompare: (target: CompareTarget) => void;
}) {
  const StatusIcon = STATUS_ICON[task.status];
  const [lastRun, setLastRun] = useState<RowLastRun | null>(null);
  const rowRef = useRef<HTMLLIElement>(null);
  const { open, handlers } = useHoverOpen(400, rowRef);

  // Track the launched run's terminal transition so the hover card can show
  // a finished duration instead of freezing on "running".
  useEffect(() => {
    if (!lastRun) return;
    const runId = lastRun.runId;
    const unlisten = onRunStatus((p) => {
      if (p.run_id !== runId) return;
      setLastRun((prev) =>
        prev && prev.runId === runId
          ? {
              ...prev,
              status: p.status,
              finishedAt: isActiveRun(p.status) ? undefined : Date.now(),
            }
          : prev,
      );
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [lastRun?.runId]);

  return (
    // tabIndex makes the row itself a keyboard stop so its rest-hidden actions
    // (revealed via :hover / :focus-within in plan.css) are reachable without
    // a pointer.
    // Until the row has launched something there is nothing behind the
    // metadata card but em dashes, so the hover handlers stay off the row and
    // the Popover stays out of the tree.
    <li
      className="task-row"
      tabIndex={0}
      ref={rowRef}
      {...(lastRun ? handlers : {})}
    >
      <div className="task-row__main">
        {/* The glyph alone carries the status at rest — the word repeats what
          * the icon and its hue already say. The visible label is revealed on
          * row hover/focus (plan.css); the screen-reader copy is always there
          * so the status never depends on a pointer. */}
        <span className={`task-status task-status--${task.status}`}>
          <StatusIcon size={16} className="task-status__icon" />
          <span className="task-status__sr">{STATUS_LABEL[task.status]}</span>
          <span className="task-status__label" aria-hidden="true">
            {STATUS_LABEL[task.status]}
          </span>
        </span>
        <span className="task-row__text">
          {taskSummary(task.text)}
        </span>
        {task.checked && (
          <span
            className="task-row__checked"
            title="Authored as checked — counts as implemented (Accepted), still runnable."
          >
            authored ✓
          </span>
        )}
        {task.run_count > 0 && (
          <button
            className="task-row__compare"
            onClick={() =>
              onCompare({
                planId,
                taskAnchor: task.anchor,
                taskText: task.text,
              })
            }
            title="Compare this task's runs and use one"
          >
            {/* The count is the always-visible readout; the "compare" cue and
              * chip chrome only surface on row hover/focus (plan.css). */}
            <span className="task-row__run-count">
              {task.run_count} {task.run_count === 1 ? "run" : "runs"}
            </span>
            <span className="task-row__compare-cue">
              {" "}
              · compare
            </span>
          </button>
        )}
      </div>
      <LaunchControl
        projectId={projectId}
        planId={planId}
        taskAnchor={task.anchor}
        installed={installed}
        agentsLoading={agentsLoading}
        onLaunched={onLaunched}
        onLaunch={(runId, agent, model, maxIterations) => {
          setLastRun({
            runId,
            agent,
            model,
            maxIterations,
            startedAt: Date.now(),
            status: "running",
          });
          onLaunch({
            runId,
            taskText: task.text,
            taskAnchor: task.anchor,
            agent,
            model,
            maxIterations,
          });
        }}
      />
      {lastRun && (
        <Popover
          open={open}
          onClose={() => {}}
          anchorRef={rowRef}
          role="dialog"
          aria-label={`${taskSummary(task.text)} details`}
          className="meta-popover"
        >
          <MetaRow
            icon={<FolderIcon size={14} />}
            value={repoName ?? "—"}
            label="Repo"
          />
          <MetaRow
            icon={<GitBranchIcon size={14} />}
            value={worktreeBranch(lastRun.runId)}
            label="Worktree branch"
          />
          <MetaRow
            icon={<AgentIcon size={14} />}
            value={lastRun.agent}
            label="Agent"
          />
          {lastRun.model && (
            <MetaRow
              icon={<AgentIcon size={14} />}
              value={lastRun.model}
              label="Model"
            />
          )}
          <MetaRow
            icon={<BoxIcon size={14} />}
            value={`${lastRun.maxIterations} ${lastRun.maxIterations === 1 ? "pass" : "passes"}`}
            label="Pass count"
          />
          <MetaRow
            icon={<ClockIcon size={14} />}
            value={
              isActiveRun(lastRun.status) ? (
                <Elapsed startedAt={lastRun.startedAt} />
              ) : lastRun.finishedAt !== undefined ? (
                `Finished in ${formatDuration(lastRun.finishedAt - lastRun.startedAt)}`
              ) : (
                RUN_STATUS_LABEL[lastRun.status]
              )
            }
            label="Elapsed or finished time"
            tone={
              isActiveRun(lastRun.status)
                ? task.status === "completed-unaccepted"
                  ? "warn"
                  : undefined
                : finishedRunTone(lastRun.status)
            }
          />
        </Popover>
      )}
    </li>
  );
}

export function LaunchControl({
  projectId,
  planId,
  taskAnchor,
  installed,
  agentsLoading = false,
  onLaunched,
  onLaunch,
  actionsPortal,
}: {
  projectId: string;
  planId: string;
  taskAnchor: string;
  installed: string[];
  agentsLoading?: boolean;
  onLaunched: () => void;
  onLaunch: (
    runId: string,
    agent: string,
    model: string,
    maxIterations: number,
  ) => void;
  /// When set, the whole control (split button, its menu, and the result
  /// message) portals there instead of rendering inline — the toolbar's
  /// action slot, wired by TaskTab.
  actionsPortal?: HTMLElement | null;
}) {
  // Empty sentinels mean "not chosen yet"; adopted once from launchPrefs
  // after the installed-agents list resolves, then left to the user.
  const [agent, setAgent] = useState<string>("");
  // Free text (with preset suggestions for Claude) — "" means the agent CLI's
  // own default model.
  const [model, setModel] = useState<string>("");
  const [passes, setPasses] = useState<number | "">("");
  const [menuOpen, setMenuOpen] = useState(false);
  const [launching, setLaunching] = useState(false);
  // Set while the pre-launch `check_agent_usage` probe is in flight, so the
  // button can say "Checking…" instead of sitting idle for however long the
  // probe takes.
  const [checking, setChecking] = useState(false);
  // Set when the probe comes back "blocked" — holds the check so the prompt
  // can show the reset time and offer scheduling against it.
  const [blockedCheck, setBlockedCheck] = useState<AgentUsageCheck | null>(
    null,
  );
  // Success swaps the button's own label briefly rather than popping a
  // floating confirmation — a Popover here would portal to document.body and
  // could land on top of the next task's row. Errors still use the Popover
  // below since they need to stay put until the user reads/dismisses them.
  const [justLaunched, setJustLaunched] = useState(false);
  const [msg, setMsg] = useState<{ text: string; ok: false } | null>(null);
  // Scheduling a start for when the limit resets is a separate action from
  // Run — it books a `scheduleLaunch` for later rather than starting now — so
  // it gets its own in-flight/confirmation state alongside `launching`.
  const [scheduling, setScheduling] = useState(false);
  const [justScheduled, setJustScheduled] = useState(false);
  // Headroom for the agent about to be launched, from the same store the
  // agents panel and the run toolbar read — the state on screen is the state
  // the run will meet.
  const { snapshots, now } = useAgentUsage();
  const initialized = useRef(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const chevronRef = useRef<HTMLButtonElement>(null);

  // The global default model (Settings), used only to seed a project that
  // has never had a model chosen for it — once `readLaunchPrefs` has a value
  // of its own, that always wins.
  const [defaultModel, setDefaultModel] = useState<string | null>(null);
  const [settingsLoaded, setSettingsLoaded] = useState(false);

  useEffect(() => {
    let cancelled = false;
    getSettings()
      .then((s) => {
        if (!cancelled) setDefaultModel(s.default_model);
      })
      .catch(() => {})
      .finally(() => {
        if (!cancelled) setSettingsLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (agentsLoading || !settingsLoaded || initialized.current) return;
    initialized.current = true;
    const prefs = readLaunchPrefs(projectId, installed);
    setAgent(prefs.agent);
    // A project with no stored model preference of its own falls back to the
    // app-wide default, but only when that default is one of the presets the
    // chosen agent actually supports — a stale value for a different agent
    // (or one the agent no longer offers) is silently dropped rather than
    // sent where it can't be honored.
    setModel(
      prefs.model ||
        (defaultModel && AGENT_MODEL_PRESETS[prefs.agent]?.includes(defaultModel)
          ? defaultModel
          : ""),
    );
    setPasses(prefs.passes);
  }, [agentsLoading, settingsLoaded, defaultModel, installed, projectId]);

  // Persist the user's choices as soon as they diverge from the loaded prefs.
  useEffect(() => {
    if (!initialized.current || agent === "" || passes === "") return;
    writeLaunchPrefs(projectId, { agent, model, passes });
  }, [projectId, agent, model, passes]);

  // A model override only makes sense for agents whose CLI supports one;
  // switching to an agent without a preset list drops it so a stray leftover
  // value never gets sent where it can't be honored.
  useEffect(() => {
    if (!AGENT_MODEL_PRESETS[agent] && model !== "") setModel("");
  }, [agent, model]);

  const noAgents = installed.length === 0;
  const passCount = passes || 1;

  // The agent this control is about to launch: the user's pick once made,
  // otherwise the stored preference the bare `Run` segment would use.
  const selectedAgent = agent || preferredAgent(projectId, installed);
  const headroom = launchHeadroom(
    selectedAgent,
    snapshots[selectedAgent] ?? null,
    now,
  );
  // Known only when the agent's own snapshot carries it — an exhausted
  // window with no reported reset instant can't be scheduled against, only
  // waited out.
  const resetAtMs = snapshots[selectedAgent]?.reset_at_ms ?? null;

  function changePasses(delta: number) {
    setPasses(Math.min(50, Math.max(1, passCount + delta)));
  }

  async function doLaunch() {
    setLaunching(true);
    setMsg(null);
    const maxIterations = Math.max(1, passes || 1);
    try {
      const runId = await launchRun({
        projectId,
        taskAnchor,
        agent,
        model: model.trim() || null,
        maxIterations,
      });
      onLaunch(runId, agent, model.trim(), maxIterations);
      onLaunched();
      setJustLaunched(true);
      setTimeout(() => setJustLaunched(false), 1500);
    } catch (e) {
      setMsg({ text: String(e), ok: false });
    } finally {
      setLaunching(false);
    }
  }

  // How long the pre-launch usage probe gets before this control gives up
  // waiting and launches anyway — set above the backend adapter probe's own
  // 20s bound (see `USAGE_PROBE_TIMEOUT`) so that timeout, not this one, is
  // normally what decides. A stuck or failed probe should never be able to
  // hold the Run button hostage.
  const USAGE_CHECK_TIMEOUT_MS = 25_000;

  async function launch() {
    setMenuOpen(false);
    setChecking(true);
    setMsg(null);
    // A probe failure or a client-side timeout both resolve to `null` here,
    // so either one falls through to "launch immediately" below exactly like
    // a "proceed" verdict — the check is a courtesy, not a gate that can wedge
    // the button.
    const check = await Promise.race<AgentUsageCheck | null>([
      checkAgentUsage(agent).catch(() => null),
      new Promise((resolve) =>
        setTimeout(() => resolve(null), USAGE_CHECK_TIMEOUT_MS),
      ),
    ]);
    setChecking(false);
    if (check && check.decision !== "proceed") {
      setBlockedCheck(check);
      return;
    }
    await doLaunch();
  }

  // Books a `scheduleLaunch` for the instant the exhausted agent's window
  // reopens, so the run fires unattended instead of the user having to
  // remember to come back and press Run. `resetAt` defaults to the headroom
  // store's figure but the blocked-launch prompt passes the just-probed
  // instant explicitly, since that snapshot may be newer than what's landed
  // in the shared store yet. Only reachable when a reset instant is known —
  // callers must guard `null` themselves.
  async function scheduleForReset(resetAt: number | null = resetAtMs) {
    if (resetAt === null) return;
    setMenuOpen(false);
    setScheduling(true);
    setMsg(null);
    try {
      await scheduleLaunch({
        planId,
        taskAnchor,
        agent: selectedAgent,
        model: model.trim() || null,
        maxIterations: passCount,
        launchAt: new Date(resetAt).toISOString(),
      });
      setJustScheduled(true);
      setTimeout(() => setJustScheduled(false), 1500);
    } catch (e) {
      setMsg({ text: String(e), ok: false });
    } finally {
      setScheduling(false);
    }
  }

  // The blocked verdict's reset instant, when the probe reported one — used
  // by the blocked-launch prompt's copy and its "schedule" action.
  const blockedResetAtMs =
    blockedCheck && blockedCheck.decision !== "proceed"
      ? blockedCheck.decision.blocked.reset_at_ms
      : null;
  // The spent window's own name (e.g. "5h", "weekly"), read off the same
  // snapshot the headroom chip uses — the probe verdict itself only carries
  // the reset instant, not which window ran out.
  const blockedWindow = snapshots[selectedAgent]?.limit_window ?? null;

  // Mirrored onto the DOM so plan.css's `:has(.launch--engaged)` can keep the
  // row visible while the menu/result popover is open — both now portal to
  // document.body via Popover, out of reach of a plain `:has(.launch__menu)`.
  const engaged =
    menuOpen ||
    msg !== null ||
    justLaunched ||
    justScheduled ||
    checking ||
    blockedCheck !== null;

  const content = (
    <div
      className={[
        "launch",
        engaged ? "launch--engaged" : "",
        headroom.warning ? "launch--exhausted" : "",
      ]
        .filter(Boolean)
        .join(" ")}
      ref={rootRef}
      title={
        noAgents ? "No agent CLI is installed" : (headroom.warning ?? undefined)
      }
    >
      <SplitButton
        className="launch__run"
        onClick={launch}
        onChevronClick={() => setMenuOpen((v) => !v)}
        chevronLabel="Choose agent and passes"
        disabled={noAgents || checking || launching || !agent}
        chevronRef={chevronRef}
      >
        {checking
          ? "Checking…"
          : launching
            ? "Launching…"
            : justLaunched
              ? "Launched"
              : "Run"}
      </SplitButton>
      <Popover
        open={menuOpen}
        onClose={() => setMenuOpen(false)}
        anchorRef={chevronRef}
        placement="bottom-end"
        role="menu"
        aria-label="Choose agent and passes"
        className="launch__menu"
      >
        <div
          className={`launch__agents${agentsLoading ? " launch__agents--loading" : ""}`}
          role="radiogroup"
          aria-label="Agent"
          aria-busy={agentsLoading}
        >
          {agentsLoading
            ? null
            : installed.map((k) => {
                // Each agent carries its own state into the pick: a spent
                // window is marked on the button itself, so choosing one isn't
                // a guess that has to be undone after reading the line below.
                const chip = usageIndicator(snapshots[k] ?? null, now);
                const classes = [
                  "launch__agent",
                  agent === k ? "launch__agent--on" : "",
                  chip.display === "exhausted"
                    ? "launch__agent--exhausted"
                    : "",
                ]
                  .filter(Boolean)
                  .join(" ");
                return (
                  <button
                    key={k}
                    type="button"
                    role="radio"
                    aria-checked={agent === k}
                    className={classes}
                    data-label={k}
                    disabled={noAgents}
                    title={chip.title}
                    onClick={() => setAgent(k)}
                  >
                    {k}
                  </button>
                );
              })}
        </div>
        {/* The selected agent's headroom in full: the figure always, and the
            consequence of launching into a spent window when there is one. */}
        {!agentsLoading && !noAgents && (
          <div
            className={`launch__usage launch__usage--${headroom.display}`}
            role="status"
          >
            <span className="launch__usage-agent">{selectedAgent}</span>
            <span className="launch__usage-figure" title={headroom.title}>
              {headroom.label}
            </span>
            {headroom.warning && (
              <span className="launch__usage-note">{headroom.warning}</span>
            )}
            {headroom.display === "exhausted" && (
              <button
                type="button"
                className="launch__schedule"
                disabled={resetAtMs === null || scheduling}
                title={
                  resetAtMs === null
                    ? "The agent hasn't reported when this window resets, so a start can't be scheduled — launch manually once it reports a limit."
                    : `Books a run to start at ${formatResetTime(resetAtMs, now)}, once the limit resets.`
                }
                onClick={() => scheduleForReset()}
              >
                {scheduling
                  ? "Scheduling…"
                  : justScheduled
                    ? "Scheduled"
                    : resetAtMs === null
                      ? "Start when the limit resets"
                      : `Start at ${formatResetTime(resetAtMs, now)}`}
              </button>
            )}
          </div>
        )}
        {AGENT_MODEL_PRESETS[agent] && (
          <div className="launch__model" role="group" aria-label="Model">
            <input
              type="text"
              list="launch-model-presets"
              className="launch__model-input"
              placeholder="Default model"
              value={model}
              disabled={noAgents}
              onChange={(e) => setModel(e.target.value)}
            />
            <datalist id="launch-model-presets">
              {AGENT_MODEL_PRESETS[agent].map((m) => (
                <option key={m} value={m} />
              ))}
            </datalist>
          </div>
        )}
        <div className="launch__count" role="group" aria-label="Maximum passes">
          <button
            type="button"
            className="launch__count-btn"
            onClick={() => changePasses(-1)}
            disabled={noAgents || passCount <= 1}
            aria-label="Decrease maximum passes"
          >
            −
          </button>
          <span className="launch__count-value" title="Maximum passes">
            {passCount}
            <span className="launch__count-label">
              {passCount === 1 ? "pass" : "passes"}
            </span>
          </span>
          <button
            type="button"
            className="launch__count-btn"
            onClick={() => changePasses(1)}
            disabled={noAgents || passCount >= 50}
            aria-label="Increase maximum passes"
          >
            +
          </button>
        </div>
      </Popover>
      <Popover
        open={msg !== null}
        onClose={() => setMsg(null)}
        anchorRef={chevronRef}
        placement="bottom-end"
        role="dialog"
        aria-label="Launch result"
        className="launch__result msg msg--err"
      >
        {msg?.text}
      </Popover>
      <Popover
        open={blockedCheck !== null}
        onClose={() => setBlockedCheck(null)}
        anchorRef={chevronRef}
        placement="bottom-end"
        role="dialog"
        aria-label="Launch blocked"
        className="launch__blocked"
      >
        <p className="launch__blocked-text">
          <strong>{selectedAgent}</strong> has used its{" "}
          {blockedWindow ? `${blockedWindow} ` : ""}limit
          {blockedResetAtMs !== null
            ? ` — it resets in ${formatCountdown(blockedResetAtMs, now)}, at ${formatResetTime(blockedResetAtMs, now)}.`
            : ", and it hasn't reported when the window reopens."}
          {blockedResetAtMs === null &&
            " A run started now will schedule its own resume once the agent reports the limit."}
        </p>
        <div className="launch__blocked-actions">
          {blockedResetAtMs !== null && (
            <button
              type="button"
              className="launch__blocked-schedule"
              onClick={() => {
                const resetAt = blockedResetAtMs;
                setBlockedCheck(null);
                scheduleForReset(resetAt);
              }}
            >
              Schedule for {formatResetTime(blockedResetAtMs, now)}
            </button>
          )}
          <button
            type="button"
            className="launch__blocked-run"
            onClick={() => {
              setBlockedCheck(null);
              doLaunch();
            }}
          >
            Run anyway
          </button>
          <button
            type="button"
            className="launch__blocked-cancel"
            onClick={() => setBlockedCheck(null)}
          >
            Cancel
          </button>
        </div>
      </Popover>
    </div>
  );

  return actionsPortal ? createPortal(content, actionsPortal) : content;
}
