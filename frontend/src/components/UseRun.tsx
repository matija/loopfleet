// "Use this run": land a run's produced output without judging it. By default it
// merges the run's final state into the repo's currently checked-out branch under
// a descriptive commit; a named branch (created if absent) lands it elsewhere.
// The run is marked accepted on success. Shared by the compare view (one column
// per run) and the run timeline (apply the run you're already looking at, no
// detour through compare). Consumes only the pre-existing `use_run` command.

import { useState } from "react";
import { useRun } from "../commands";
import type { UseRunResult } from "../types";

export function UseRun({
  runId,
  mergeable,
  onAccepted,
}: {
  runId: string;
  /// True when the run produced a snapshot to merge (a final iteration ref).
  mergeable: boolean;
  onAccepted: () => void;
}) {
  const [branch, setBranch] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<UseRunResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const custom = branch.trim() !== "";

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
        <button
          className="btn btn--accent use-run__go"
          onClick={apply}
          disabled={!mergeable || busy}
          title={!mergeable ? "No snapshot to merge" : undefined}
        >
          {busy ? "Merging…" : "Use this run"}
        </button>
      </div>
      <p className="use-run__hint">
        {custom ? (
          <>
            Merges into <code>{branch.trim()}</code>.
          </>
        ) : (
          <>Merges into your current branch.</>
        )}
      </p>
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
      {error && <p className="use-run__error">{error}</p>}
    </div>
  );
}
