// Expanded state for individual tasks, keyed by plan id + task anchor so
// expansion follows a task across a re-parse rather than a list index.
// Persisted as a single { [taskKey]: true } map under one localStorage key,
// mirroring sidebarCollapse.ts.

import { useCallback, useState } from "react";

const STORAGE_KEY = "loopfleet.task.expanded";

function readAll(): Record<string, boolean> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? JSON.parse(raw) : {};
  } catch {
    return {};
  }
}

function writeAll(state: Record<string, boolean>) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    // localStorage unavailable (private mode, quota) — expansion state just
    // doesn't persist across reloads.
  }
}

/// Pure toggle: flips `key`'s presence in `state`, returning a new map.
/// Absence means collapsed, so toggling off deletes the key rather than
/// storing `false`.
export function toggleKey(
  state: Record<string, boolean>,
  key: string,
): Record<string, boolean> {
  const next = { ...state };
  if (next[key]) delete next[key];
  else next[key] = true;
  return next;
}

/// Tracks whether the task `key` (plan id + task anchor) is expanded,
/// persisting toggles to localStorage under the shared
/// `loopfleet.task.expanded` key.
export function useTaskExpanded(key: string): [boolean, () => void] {
  const [expanded, setExpanded] = useState(() => readAll()[key] === true);

  const toggle = useCallback(() => {
    const all = toggleKey(readAll(), key);
    writeAll(all);
    setExpanded(all[key] === true);
  }, [key]);

  return [expanded, toggle];
}
