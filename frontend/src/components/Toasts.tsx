// Transient command-error surface (PRD M7 polish: "toast for command errors").
// Command failures that aren't tied to a specific form field — stopping a run,
// launching, the initial project load — used to overwrite a single persistent
// banner. Here they stack, auto-dismiss, and can be closed. Contextual form
// errors (add-project, settings save) stay inline next to their control; this is
// only for the app-level command lane App already funneled into one banner.

import { useCallback, useRef, useState } from "react";

export type Toast = { id: number; message: string };

// Auto-dismiss after this long; a lingering banner reads as broken, a toast that
// clears itself reads as a passing failure the user can still catch.
const DISMISS_MS = 6000;

export function useToasts() {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const nextId = useRef(1);

  const dismiss = useCallback((id: number) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const push = useCallback(
    (message: string) => {
      const id = nextId.current++;
      setToasts((prev) => [...prev, { id, message }]);
      setTimeout(() => dismiss(id), DISMISS_MS);
    },
    [dismiss],
  );

  return { toasts, push, dismiss };
}

export function Toasts({
  toasts,
  onDismiss,
}: {
  toasts: Toast[];
  onDismiss: (id: number) => void;
}) {
  if (toasts.length === 0) return null;
  return (
    // aria-live=assertive: a command failing is worth interrupting a screen
    // reader for. Fixed overlay so it floats above the main pane and the dock.
    <div className="toasts" role="region" aria-label="Errors" aria-live="assertive">
      {toasts.map((t) => (
        <div key={t.id} className="toast" role="alert">
          <span className="toast__msg">{t.message}</span>
          <button
            className="toast__close"
            onClick={() => onDismiss(t.id)}
            aria-label="Dismiss"
          >
            ✕
          </button>
        </div>
      ))}
    </div>
  );
}
