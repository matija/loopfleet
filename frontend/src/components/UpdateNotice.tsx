import { useCallback, useEffect, useRef, useState } from "react";
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
  const checking = useRef(false);

  useEffect(() => {
    if (import.meta.env.DEV) return;
    let cancelled = false;
    check({ timeout: 15_000 })
      .then((update) => {
        if (!cancelled && update?.available) {
          setState({ phase: "available", update });
        }
      })
      .catch((err) => {
        if (!cancelled) console.warn("Background update check failed", err);
      });
    return () => {
      cancelled = true;
    };
  }, []);

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

  const checkNow = useCallback(async () => {
    if (checking.current || state.phase === "installing") return;
    checking.current = true;
    setState({ phase: "checking" });
    try {
      const update = await check({ timeout: 15_000 });
      setState(
        update?.available ? { phase: "available", update } : { phase: "current" },
      );
    } catch (err) {
      setState({ phase: "idle" });
      console.warn("Manual update check failed", err);
      onError("Could not check for updates. Check your connection and try again.");
    } finally {
      checking.current = false;
    }
  }, [onError, state.phase]);

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
