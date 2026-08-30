// The global keyboard-shortcut registry: one static list of every shortcut
// the app recognizes window-wide, and the pure matcher that turns a keystroke
// into a ShortcutId. Prior art: themes.ts (a registry plus pure functions over
// it, with the DOM touched in exactly one place) and typeahead.ts/optionIndex.ts
// (matching against a minimal event shape instead of a real KeyboardEvent, so
// the logic is testable without a DOM).
//
// This module only answers "which shortcut, if any, does this keystroke
// mean" — the effect a matched id triggers (opening the palette, toggling the
// sidebar, …) stays wherever it already lived.
//
// Escape is deliberately left out. It's layer-local: whichever surface is
// topmost (Popover, CommandPalette) owns closing itself on Escape, and each
// already wires its own listener for that. A global registry entry for
// Escape would have no way to know which surface is topmost, so it would
// risk closing the wrong one.

export type ShortcutId = "commandPalette" | "toggleSidebar" | "openSettings";

/// "mod" is the platform's primary modifier: Cmd on macOS, Ctrl elsewhere.
/// Matching accepts either key for it (mirroring the app's existing
/// `e.metaKey || e.ctrlKey` checks) rather than sniffing the platform, since
/// either produces one unambiguous, testable match.
export type Modifier = "mod" | "shift" | "alt";

export type Shortcut = {
  id: ShortcutId;
  /// Matched case-insensitively against KeyboardEvent.key.
  key: string;
  /// Modifiers that must be held. Any modifier not listed must be up — a
  /// shortcut matches one exact combination, never a superset of it. A
  /// shortcut with no modifiers is skipped while the event target is
  /// editable (a form field or contenteditable), so it never steals an
  /// ordinary keystroke from whatever the user is typing; a shortcut with at
  /// least one modifier always fires.
  mods: readonly Modifier[];
  /// Human-readable description, for a future shortcuts help panel.
  label: string;
};

/// Both entries mirror shortcuts App.tsx already wires up directly (⌘K/Ctrl-K
/// for the palette, ⌘B/Ctrl-B for the sidebar); this registry is the seam
/// later shortcuts hang off instead of each growing its own `keydown` effect.
export const SHORTCUTS: readonly Shortcut[] = [
  {
    id: "commandPalette",
    key: "k",
    mods: ["mod"],
    label: "Open command palette",
  },
  {
    id: "toggleSidebar",
    key: "b",
    mods: ["mod"],
    label: "Toggle sidebar",
  },
  {
    id: "openSettings",
    key: ",",
    mods: ["mod"],
    label: "Open run defaults",
  },
] as const;

const MODIFIER_GLYPH: Record<Modifier, string> = {
  mod: "⌘",
  shift: "⇧",
  alt: "⌥",
};

/// The glyphs to render for a shortcut, one `<kbd>` per element: its
/// modifiers in registry order, then the key itself. Used by the command
/// palette's footer hint row (and any future shortcuts help panel) so the
/// on-screen keys stay in sync with the registry instead of being retyped.
export function shortcutKeyGlyphs(shortcut: Shortcut): string[] {
  return [...shortcut.mods.map((m) => MODIFIER_GLYPH[m]), shortcut.key.toUpperCase()];
}

/// The minimal shape matchShortcut needs from a keystroke: the modifier keys
/// and character a KeyboardEvent already carries, plus whether the target is
/// editable (a KeyboardEvent alone doesn't say that — see `isEditableTarget`).
/// Kept separate from KeyboardEvent so matching is testable without a DOM.
export type ShortcutEvent = {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  editable: boolean;
};

function modsMatch(event: ShortcutEvent, mods: readonly Modifier[]): boolean {
  const mod = event.metaKey || event.ctrlKey;
  return (
    mod === mods.includes("mod") &&
    event.shiftKey === mods.includes("shift") &&
    event.altKey === mods.includes("alt")
  );
}

/// The shortcut `event` invokes, if any. Checked in registry order, first
/// match wins; a modifier-less entry is skipped while `event.editable` is
/// true, as if absent, so it never steals an ordinary keystroke from a form
/// field or contenteditable.
export function matchShortcut(
  event: ShortcutEvent,
  shortcuts: readonly Shortcut[] = SHORTCUTS,
): ShortcutId | null {
  const match = shortcuts.find(
    (s) =>
      s.key.toLowerCase() === event.key.toLowerCase() &&
      modsMatch(event, s.mods) &&
      (s.mods.length > 0 || !event.editable),
  );
  return match?.id ?? null;
}

/// True when `target` consumes ordinary keystrokes as text: a form field or
/// anything marked contenteditable. Duck-typed on the shape rather than
/// `instanceof HTMLElement` so it works the same against a real DOM node and
/// a plain test double.
export function isEditableTarget(
  target: { tagName?: string; isContentEditable?: boolean } | null | undefined,
): boolean {
  if (!target) return false;
  if (target.isContentEditable) return true;
  return target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.tagName === "SELECT";
}

/// Converts a real KeyboardEvent into the minimal shape matchShortcut needs,
/// resolving editability from the event's target. The one place this module
/// touches the DOM — everything else matches against `ShortcutEvent`, a plain
/// object.
export function shortcutEventFromDOM(event: KeyboardEvent): ShortcutEvent {
  return {
    key: event.key,
    metaKey: event.metaKey,
    ctrlKey: event.ctrlKey,
    shiftKey: event.shiftKey,
    altKey: event.altKey,
    editable: isEditableTarget(
      event.target as { tagName?: string; isContentEditable?: boolean } | null,
    ),
  };
}
