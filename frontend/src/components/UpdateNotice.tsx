// Update affordance (PRD M7: in-app update). Checks once on mount, and again
// whenever the app menu's "Check for Updates…" fires. If the updater endpoint
// reports a newer build, offers to download, install and relaunch. Failures at
// any step (check, download, install) are reported through the caller's toast
// surface rather than a blocking dialog — a stale build is not worth
// interrupting the user's work over.
//
// The two entry points differ in what silence means. The launch check is
// unsolicited, so "you are up to date" stays invisible; a menu check was asked
// for, so it answers either way.

import { useCallback, useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

type UpdateState =
  | { phase: "idle" }
  | { phase: "checking" }
  | { phase: "current" }
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

  // The menu-driven check. Unlike the launch one it narrates itself: the user
  // asked, so "checking…" and "you are on the latest version" are both answers
  // worth showing. A check already in flight is left alone.
  const checkNow = useCallback(async () => {
    setState((prev) =>
      prev.phase === "checking" || prev.phase === "installing"
        ? prev
        : { phase: "checking" },
    );
    try {
      const update = await check();
      setState(
        update?.available ? { phase: "available", update } : { phase: "current" },
      );
    } catch (err) {
      setState({ phase: "idle" });
      onError(`Update check failed: ${errorMessage(err)}`);
    }
  }, [onError]);

  const dismiss = useCallback(() => setState({ phase: "idle" }), []);

  return { state, install, checkNow, dismiss };
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
  const offering = state.phase === "available";

  return (
    <div className="update-notice" role="status">
      <span className="update-notice__msg">
        {state.phase === "checking"
          ? "Checking for updates…"
          : state.phase === "current"
            ? "Loopfleet is up to date."
            : installing
              ? `Installing update ${state.update.version}…`
              : `Update ${state.update.version} is available.`}
      </span>
      {offering && (
        <button className="update-notice__action" onClick={onInstall}>
          Download &amp; Install
        </button>
      )}
      <button
        className="update-notice__dismiss"
        onClick={onDismiss}
        disabled={installing || state.phase === "checking"}
        aria-label="Dismiss"
      >
        ✕
      </button>
    </div>
  );
}
