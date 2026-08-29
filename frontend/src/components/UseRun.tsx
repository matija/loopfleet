// "Use this run": land a run's produced output without judging it. By default it
// merges the run's final state into the repo's currently checked-out branch under
// a descriptive commit; a named branch (created if absent) lands it elsewhere.
// The run is marked accepted on success. Shared by the compare view (one column
// per run) and the run timeline (apply the run you're already looking at, no
// detour through compare). Consumes only the pre-existing `use_run` command.

import { useState } from "react";
import { createPortal } from "react-dom";
import { useRun } from "../commands";
import type { UseRunResult } from "../types";
import { DoubleCheckIcon } from "./Icon";

export function UseRun({
  runId,
  mergeable,
  accepted,
  onAccepted,
  actionsPortal,
}: {
  runId: string;
  /// True when the run produced a snapshot to merge (a final iteration ref).
  mergeable: boolean;
  /// True once the run has already landed (`use_run` accepted it). Same
  /// three-state split as the dock chip: merged (static marker, no live
  /// control), mergeable (the button), or neither (disabled, nothing to
  /// land) — a landed run must not keep offering a control that would merge
  /// it again.
  accepted: boolean;
  onAccepted: () => void;
  /// When set, the "Use this run" button (or the merged marker, once landed)
  /// portals there instead of rendering inline — the toolbar's action slot
  /// (RunTimeline's "Use run", CompareView's "Accept"). The branch input,
  /// hint, result, and error stay inline.
  actionsPortal?: HTMLElement | null;
}) {
  const [branch, setBranch] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<UseRunResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const custom = branch.trim() !== "";
  const useButton = accepted ? (
    <span
      className="use-run__merged"
      role="img"
      aria-label="Merged"
      title="Already merged into your branch"
    >
      <DoubleCheckIcon size={14} />
    </span>
  ) : (
    <button
      className="btn btn--primary use-run__go"
      onClick={apply}
      disabled={!mergeable || busy}
      title={!mergeable ? "No snapshot to merge" : undefined}
    >
      {busy ? "Merging…" : "Use this run"}
    </button>
  );

  async function apply() {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const r = await useRun(runId, custom ? branch.trim() : null);
      setResult(r);
      onAccepted();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="use-run">
      <div className="use-run__row">
        {!accepted && (
          <input
            className="use-run__branch"
            type="text"
            placeholder="current branch (optional)"
            value={branch}
            disabled={!mergeable || busy}
            onChange={(e) => setBranch(e.target.value)}
            aria-label="Target branch"
            title="Leave empty to merge into your current branch. Name a branch to land the run elsewhere."
          />
        )}
        {actionsPortal ? createPortal(useButton, actionsPortal) : useButton}
      </div>
      {!accepted && (
        <p className="use-run__hint">
          {custom ? (
            <>
              Merges into <code>{branch.trim()}</code>.
            </>
          ) : (
            <>Merges into your current branch.</>
          )}
        </p>
      )}
      {result && (
        <p className="use-run__result">
          Merged into <code>{result.target_branch}</code>{" "}
          {result.up_to_date
            ? "(already up to date)"
            : result.created
              ? "(branch created)"
              : `→ ${result.merged_commit.slice(0, 8)}`}
        </p>
      )}
      {result?.cleanup_error && (
        <p className="use-run__error">Cleanup after merge failed: {result.cleanup_error}</p>
      )}
      {error && <p className="use-run__error">{error}</p>}
    </div>
  );
}
