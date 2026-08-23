// The theme registry. tokens.css defines one color block per theme, selected
// by [data-theme="<id>"] on the document root; this module is the TypeScript
// side of that contract — the list of ids a picker can offer, the validation a
// stored id has to pass, and the one place that writes the attribute.
//
// Adding a theme means adding its block to tokens.css and one entry here.
// Nothing else in the app should hardcode a theme id or touch data-theme.

/// Ids must match the [data-theme="…"] blocks in tokens.css exactly.
export type ThemeId = "dark" | "rose-pine-moon";

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

/// Applies a theme by setting data-theme on the document root, resolving the
/// value first so a stale stored id degrades to the default rather than
/// leaving the root with an attribute tokens.css has no block for (which would
/// inherit no color tokens at all). Returns the id actually applied.
///
/// `root` defaults to document.documentElement; pass one explicitly to theme a
/// detached tree or to test without a DOM.
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
