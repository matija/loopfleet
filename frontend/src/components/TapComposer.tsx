import { useRef, useState, type RefObject } from "react";
import { tapRun } from "../commands";
import { Button } from "./Button";
import { Popover } from "./Popover";
import type { ActiveRun } from "./RunDock";

export const isValidTapMessage = (text: string): boolean => text.trim() !== "";

export function TapComposer({ run, anchorRef, onClose }: {
  run: Pick<ActiveRun, "runId" | "taskText" | "projectName">;
  anchorRef: RefObject<HTMLElement | null>;
  onClose: () => void;
}) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sending = useRef(false);

  async function send() {
    if (!isValidTapMessage(text) || sending.current) return;
    sending.current = true;
    setBusy(true);
    setError(null);
    try {
      await tapRun(run.runId, text);
      onClose();
      anchorRef.current?.focus();
    } catch (e) {
      setError(String(e));
    } finally {
      sending.current = false;
      setBusy(false);
    }
  }

  return (
    <Popover open anchorRef={anchorRef} onClose={onClose} role="dialog" aria-label={`Tap ${run.taskText} — ${run.projectName}`}>
      <form onSubmit={(e) => { e.preventDefault(); void send(); }} style={{ width: "min(360px, calc(100vw - 32px))", display: "grid", gap: "var(--space-2)" }}>
        <strong style={{ overflowWrap: "anywhere" }}>{run.taskText}</strong>
        <span style={{ overflowWrap: "anywhere" }}>{run.projectName}</span>
        <textarea
          autoFocus
          rows={4}
          aria-label="Message to the run"
          placeholder="Send guidance to this run…"
          value={text}
          disabled={busy}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void send();
            }
          }}
          style={{ resize: "vertical", font: "inherit", padding: "var(--space-2)", color: "var(--c-text)", background: "var(--c-surface-2)", border: "1px solid var(--c-border)", borderRadius: "var(--radius-sm)" }}
        />
        <span>Enter to send · Shift+Enter for a new line</span>
        {error && <p role="alert" style={{ color: "var(--c-danger)", margin: 0 }}>{error}</p>}
        <Button variant="primary" type="submit" disabled={busy || !isValidTapMessage(text)}>{busy ? "Sending…" : "Send"}</Button>
      </form>
    </Popover>
  );
}
