// Collapsed/expanded state for the app's disclosures: sidebar sections,
// plan-tree group headers, and panels that open closed (the plan pane's
// sandbox overrides). Persisted as a single { [groupId]: boolean } map under
// one localStorage key so every disclosure shares one place to look up and
// survive a reload.
//
// Only states that differ from the disclosure's own default are stored, and a
// toggle back to the default deletes the entry: the map stays a list of the
// user's deliberate departures rather than a row per disclosure ever touched.

import { useCallback, useState } from "react";

const STORAGE_KEY = "loopfleet.sidebar.collapsed";

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
    // localStorage unavailable (private mode, quota) — collapse state just
    // doesn't persist across reloads.
  }
}

/// Tracks whether the group `id` is collapsed, persisting toggles to
/// localStorage under the shared `loopfleet.sidebar.collapsed` key.
/// `defaultCollapsed` is the state before the user has touched this
/// disclosure — true for a panel that should open closed.
export function useSidebarCollapsed(
  id: string,
  defaultCollapsed = false,
): [boolean, () => void] {
  const [collapsed, setCollapsed] = useState(
    () => readAll()[id] ?? defaultCollapsed,
  );

  const toggle = useCallback(() => {
    setCollapsed((prev) => {
      const next = !prev;
      const all = readAll();
      if (next === defaultCollapsed) delete all[id];
      else all[id] = next;
      writeAll(all);
      return next;
    });
  }, [id, defaultCollapsed]);

  return [collapsed, toggle];
}
