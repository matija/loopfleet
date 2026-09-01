// The theme registry. tokens.css defines one color block per theme, selected
// by [data-theme="<id>"]; this module is the TypeScript side of that contract
// — the list of ids a picker can offer, the validation a stored id has to
// pass, and the one place that writes the attribute.
//
// The attribute is normally worn by the document root, but the tokens.css
// blocks match any element, so a subtree can be painted in a different theme
// than the app is wearing (that's how the settings panel previews a theme
// before it's picked — see ThemePreview.tsx).
//
// Adding a theme means adding its block to tokens.css and one entry here.
// Nothing else in the app should hardcode a theme id or touch data-theme.

/// Ids must match the [data-theme="…"] blocks in tokens.css exactly.
export type ThemeId =
  | "dark"
  | "rose-pine-moon"
  | "github-dark"
  | "github-light"
  | "tokyo-night"
  | "tokyo-night-storm"
  | "tokyo-night-light"
  | "tairiki-dark"
  | "tairiki-light"
  | "dracula";

export type Theme = {
  id: ThemeId;
  /// Human-readable name for the theme picker.
  label: string;
};

/// dark is first because it's the default — tokens.css also applies it via
/// :root with no attribute set, so it's what the app looks like before any
/// preference is read.
export const THEMES: readonly Theme[] = [
  { id: "dark", label: "Dark" },
  { id: "rose-pine-moon", label: "Rosé Pine Moon" },
  { id: "github-dark", label: "GitHub Dark" },
  { id: "github-light", label: "GitHub Light" },
  { id: "tokyo-night", label: "Tokyo Night" },
  { id: "tokyo-night-storm", label: "Tokyo Night Storm" },
  { id: "tokyo-night-light", label: "Tokyo Night Light" },
  { id: "tairiki-dark", label: "Tairiki Dark" },
  { id: "tairiki-light", label: "Tairiki Light" },
  { id: "dracula", label: "Dracula" },
] as const;

export const DEFAULT_THEME_ID: ThemeId = "dark";

/// Narrows an arbitrary value to a known theme id.
export function isThemeId(value: unknown): value is ThemeId {
  return THEMES.some((theme) => theme.id === value);
}

/// Resolves a stored preference to a usable theme id, falling back to the
/// default when the value is absent, not a string, or names a theme that no
/// longer exists (e.g. it was renamed or removed since the user picked it).
export function resolveThemeId(stored: unknown): ThemeId {
  return isThemeId(stored) ? stored : DEFAULT_THEME_ID;
}

/// Looks up the registry entry for `id`.
export function themeById(id: ThemeId): Theme {
  // Non-null: ThemeId is exactly the set of ids in THEMES.
  return THEMES.find((theme) => theme.id === id)!;
}

/// The theme a preview should paint while a picker is being browsed: the
/// option currently under the cursor/keyboard if there is one, otherwise the
/// theme in force. Kept here rather than in the picker so "what am I looking
/// at" is one resolved id, and so a highlighted value that isn't a live theme
/// id degrades to the applied theme instead of an unpainted box.
export function previewThemeId(highlighted: unknown, applied: ThemeId): ThemeId {
  return isThemeId(highlighted) ? highlighted : applied;
}

/// Applies a theme by setting data-theme on an element, resolving the
/// value first so a stale stored id degrades to the default rather than
/// leaving the root with an attribute tokens.css has no block for (which would
/// inherit no color tokens at all). Returns the id actually applied.
///
/// `root` defaults to document.documentElement; pass one explicitly to theme a
/// subtree, a detached tree, or to test without a DOM.
export function applyTheme(
  stored: unknown,
  root: Element | undefined = typeof document === "undefined"
    ? undefined
    : document.documentElement,
): ThemeId {
  const id = resolveThemeId(stored);
  root?.setAttribute("data-theme", id);
  return id;
}

/// Where the picked theme is persisted. The inline bootstrap script in
/// index.html reads this same key before the bundle loads — keep the two in
/// sync (that script is the only other place allowed to know it).
export const THEME_STORAGE_KEY = "loopfleet.theme";

/// Reads the persisted theme id, degrading to the default when nothing is
/// stored, the value names an unknown theme, or localStorage is unavailable
/// (private mode, disabled storage).
export function readStoredThemeId(): ThemeId {
  try {
    return resolveThemeId(localStorage.getItem(THEME_STORAGE_KEY));
  } catch {
    return DEFAULT_THEME_ID;
  }
}

/// Persists the picked theme. A storage failure is swallowed: the theme still
/// applies for this session, it just doesn't survive a reload.
export function storeThemeId(id: ThemeId): void {
  try {
    localStorage.setItem(THEME_STORAGE_KEY, id);
  } catch {
    // localStorage unavailable (private mode, quota) — the pick just doesn't
    // persist across reloads.
  }
}
