// The export affordance: one quiet secondary control, reused wherever a report
// can be produced (a task's detail view, a plan header, a PRD document header).
//
// Exporting is a side errand, never the reason a surface exists, so the control
// stays subdued — an outline chip that only warms on hover, next to the primary
// actions rather than competing with them. The native save dialog is the real
// interface; this is just the door to it.
//
// The backend commands resolve to the saved path, or `null` when the user
// cancels the dialog. Cancelling is not a failure and says nothing; a genuine
// error goes to the app-level toast stack (`onError`), which is where every
// other non-form command failure already lands.

import { useState } from "react";
import { DownloadIcon } from "./Icon";

export function ExportButton({
  label = "Export",
  title,
  onExport,
  onError,
  disabled = false,
}: {
  /// Visible label. Defaults to "Export"; callers name the subject when the
  /// surface hosts more than one export ("Export plan").
  label?: string;
  title?: string;
  /// Runs the export command. Resolves to the saved path, or `null` if the
  /// user cancelled the save dialog.
  onExport: () => Promise<string | null>;
  /// Surfaces a failed export through the app's toast stack.
  onError: (message: string) => void;
  disabled?: boolean;
}) {
  const [busy, setBusy] = useState(false);

  async function run() {
    if (busy) return;
    setBusy(true);
    try {
      await onExport();
    } catch (e) {
      onError(`Export failed: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <button
      className="export-btn"
      onClick={run}
      disabled={disabled || busy}
      title={title}
    >
      <DownloadIcon size={14} className="export-btn__icon" />
      {busy ? "Exporting…" : label}
    </button>
  );
}
