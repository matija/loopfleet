// Live "elapsed" stamp for a running run: how long it has been going, ticking
// once a second against its captured launch time. Mirrors CommandBar's
// `Freshness` pattern; reuses the grid's `formatDuration` so the wall-clock
// span reads the same everywhere.

import { useEffect, useState } from "react";
import { formatDuration } from "./DataGrid";

/// Elapsed since `startedAt` (epoch ms), re-rendering each second so the value
/// climbs while the run is live. Render only for active runs — with no end time
/// there is nothing to freeze a finished run's total against.
export function Elapsed({ startedAt }: { startedAt: number }) {
  const [, tick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(id);
  }, []);
  return (
    <span className="run-elapsed" aria-label="Elapsed time">
      {formatDuration(Date.now() - startedAt)}
    </span>
  );
}
