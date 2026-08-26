// Launch-time update check (PRD M7: in-app update affordance). Checks once on
// mount; if the updater endpoint reports a newer build, offers to download,
// install and relaunch. Failures at any step (check, download, install) are
// reported through the caller's toast surface rather than a blocking dialog —
// a stale build is not worth interrupting the user's work over.

import { useCallback, useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

type UpdateState =
  | { phase: "idle" }
  | { phase: "available"; update: Update }
  | { phase: "installing"; update: Update };

export function useAppUpdater(onError: (message: string) => void) {
  const [state, setState] = useState<UpdateState>({ phase: "idle" });

  useEffect(() => {
    let cancelled = false;
    check()
      .then((update) => {
        if (!cancelled && update?.available) {
          setState({ phase: "available", update });
        }
      })
      .catch((err) => onError(`Update check failed: ${errorMessage(err)}`));
    return () => {
      cancelled = true;
    };
  }, [onError]);

  const install = useCallback(async () => {
    if (state.phase !== "available") return;
    const { update } = state;
    setState({ phase: "installing", update });
    try {
      await update.downloadAndInstall();
      await relaunch();
    } catch (err) {
      onError(`Update install failed: ${errorMessage(err)}`);
      setState({ phase: "available", update });
    }
  }, [state, onError]);

  const dismiss = useCallback(() => setState({ phase: "idle" }), []);

  return { state, install, dismiss };
}

function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function UpdateNotice({
  state,
  onInstall,
  onDismiss,
}: {
  state: UpdateState;
  onInstall: () => void;
  onDismiss: () => void;
}) {
  if (state.phase === "idle") return null;
  const installing = state.phase === "installing";

  return (
    <div className="update-notice" role="status">
      <span className="update-notice__msg">
        {installing
          ? `Installing update ${state.update.version}…`
          : `Update ${state.update.version} is available.`}
      </span>
      {!installing && (
        <button className="update-notice__action" onClick={onInstall}>
          Download &amp; Install
        </button>
      )}
      <button
        className="update-notice__dismiss"
        onClick={onDismiss}
        disabled={installing}
        aria-label="Dismiss"
      >
        ✕
      </button>
    </div>
  );
}
