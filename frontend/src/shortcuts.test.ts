import { describe, expect, it } from "vitest";
import {
  isEditableTarget,
  matchShortcut,
  shortcutEventFromDOM,
  SHORTCUTS,
  type Shortcut,
  type ShortcutEvent,
} from "./shortcuts";

function event(overrides: Partial<ShortcutEvent> = {}): ShortcutEvent {
  return {
    key: "k",
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    editable: false,
    ...overrides,
  };
}

describe("SHORTCUTS", () => {
  it("has unique ids and non-empty labels", () => {
    const ids = SHORTCUTS.map((s) => s.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const s of SHORTCUTS) {
      expect(s.label.length).toBeGreaterThan(0);
    }
  });

  it("has no two entries sharing a chord", () => {
    const chords = SHORTCUTS.map(
      (s) => `${[...s.mods].sort().join("+")}:${s.key.toLowerCase()}`,
    );
    expect(new Set(chords).size).toBe(chords.length);
  });
});

describe("matchShortcut", () => {
  it("matches the mod+key combination against the registry", () => {
    expect(matchShortcut(event({ key: "k", metaKey: true }))).toBe(
      "commandPalette",
    );
    expect(matchShortcut(event({ key: "k", ctrlKey: true }))).toBe(
      "commandPalette",
    );
    expect(matchShortcut(event({ key: "b", metaKey: true }))).toBe(
      "toggleSidebar",
    );
    expect(matchShortcut(event({ key: ",", metaKey: true }))).toBe(
      "openSettings",
    );
  });

  it("is case-insensitive on the key", () => {
    expect(matchShortcut(event({ key: "K", metaKey: true }))).toBe(
      "commandPalette",
    );
  });

  it("returns null when no modifier is held", () => {
    expect(matchShortcut(event({ key: "k" }))).toBeNull();
  });

  it("returns null when an unlisted modifier is also held", () => {
    expect(
      matchShortcut(event({ key: "k", metaKey: true, shiftKey: true })),
    ).toBeNull();
    expect(
      matchShortcut(event({ key: "k", metaKey: true, altKey: true })),
    ).toBeNull();
  });

  it("returns null when nothing matches the key", () => {
    expect(matchShortcut(event({ key: "z", metaKey: true }))).toBeNull();
  });

  it("skips a modifier-less shortcut while the target is editable", () => {
    const custom: Shortcut[] = [
      {
        id: "commandPalette",
        key: "/",
        mods: [],
        label: "Focus search",
      },
    ];
    expect(
      matchShortcut(event({ key: "/", editable: true }), custom),
    ).toBeNull();
    expect(
      matchShortcut(event({ key: "/", editable: false }), custom),
    ).toBe("commandPalette");
  });

  it("a modifier shortcut fires even while the target is editable", () => {
    expect(
      matchShortcut(event({ key: "k", metaKey: true, editable: true })),
    ).toBe("commandPalette");
    expect(
      matchShortcut(event({ key: ",", metaKey: true, editable: true })),
    ).toBe("openSettings");
  });

  it("does not open settings when a bare comma is typed into an editable field", () => {
    expect(
      matchShortcut(event({ key: ",", editable: true })),
    ).toBeNull();
  });

  it("checks the registry in order and returns the first match", () => {
    const custom: Shortcut[] = [
      { id: "commandPalette", key: "k", mods: ["mod"], label: "First" },
      { id: "toggleSidebar", key: "k", mods: ["mod"], label: "Second" },
    ];
    expect(matchShortcut(event({ key: "k", metaKey: true }), custom)).toBe(
      "commandPalette",
    );
  });
});

describe("isEditableTarget", () => {
  it("treats form fields as editable", () => {
    expect(isEditableTarget({ tagName: "INPUT" })).toBe(true);
    expect(isEditableTarget({ tagName: "TEXTAREA" })).toBe(true);
    expect(isEditableTarget({ tagName: "SELECT" })).toBe(true);
  });

  it("treats contenteditable elements as editable regardless of tag", () => {
    expect(isEditableTarget({ tagName: "DIV", isContentEditable: true })).toBe(
      true,
    );
  });

  it("treats everything else as not editable", () => {
    expect(isEditableTarget({ tagName: "DIV" })).toBe(false);
    expect(isEditableTarget({ tagName: "BUTTON" })).toBe(false);
    expect(isEditableTarget(null)).toBe(false);
    expect(isEditableTarget(undefined)).toBe(false);
  });
});

describe("shortcutEventFromDOM", () => {
  it("carries the modifier keys and key through unchanged", () => {
    const fakeEvent = {
      key: "k",
      metaKey: true,
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
      target: { tagName: "DIV" },
    } as unknown as KeyboardEvent;

    expect(shortcutEventFromDOM(fakeEvent)).toEqual({
      key: "k",
      metaKey: true,
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
      editable: false,
    });
  });

  it("resolves editable from the event's target", () => {
    const fakeEvent = {
      key: "k",
      metaKey: true,
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
      target: { tagName: "INPUT" },
    } as unknown as KeyboardEvent;

    expect(shortcutEventFromDOM(fakeEvent).editable).toBe(true);
  });
});
