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

export type ShortcutId = "commandPalette" | "toggleSidebar";

/// "mod" is the platform's primary modifier: Cmd on macOS, Ctrl elsewhere.
/// Matching accepts either key for it (mirroring the app's existing
/// `e.metaKey || e.ctrlKey` checks) rather than sniffing the platform, since
/// either produces one unambiguous, testable match.
export type Modifier = "mod" | "shift" | "alt";

export type ShortcutWhen =
  /// Fires no matter what's focused.
  | "always"
  /// Fires everywhere except while the event target is editable (a form
  /// field or contenteditable) — for shortcuts that would otherwise steal an
  /// ordinary keystroke from whatever the user is typing.
  | "not-editable";

export type Shortcut = {
  id: ShortcutId;
  /// Matched case-insensitively against KeyboardEvent.key.
  key: string;
  /// Modifiers that must be held. Any modifier not listed must be up — a
  /// shortcut matches one exact combination, never a superset of it.
  mods: readonly Modifier[];
  /// Human-readable description, for a future shortcuts help panel.
  label: string;
  when: ShortcutWhen;
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
    when: "always",
  },
  {
    id: "toggleSidebar",
    key: "b",
    mods: ["mod"],
    label: "Toggle sidebar",
    when: "always",
  },
] as const;

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
/// match wins; an entry whose `when` excludes the current target (e.g.
/// `not-editable` while `event.editable` is true) is skipped as if absent.
export function matchShortcut(
  event: ShortcutEvent,
  shortcuts: readonly Shortcut[] = SHORTCUTS,
): ShortcutId | null {
  const match = shortcuts.find(
    (s) =>
      s.key.toLowerCase() === event.key.toLowerCase() &&
      modsMatch(event, s.mods) &&
      (s.when === "always" || !event.editable),
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
