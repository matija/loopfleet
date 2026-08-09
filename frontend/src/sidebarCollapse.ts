// Collapsed/expanded state for sidebar section and plan-tree group headers.
// Persisted as a single { [groupId]: true } map under one localStorage key so
// every disclosure in the sidebar (the "Projects" section, each plan group)
// shares one place to look up and survive a reload.

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
export function useSidebarCollapsed(id: string): [boolean, () => void] {
  const [collapsed, setCollapsed] = useState(() => readAll()[id] === true);

  const toggle = useCallback(() => {
    setCollapsed((prev) => {
      const next = !prev;
      const all = readAll();
      if (next) all[id] = true;
      else delete all[id];
      writeAll(all);
      return next;
    });
  }, [id]);

  return [collapsed, toggle];
}
