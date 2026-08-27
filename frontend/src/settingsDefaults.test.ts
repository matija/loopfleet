import { describe, expect, it } from "vitest";
import {
  DEFAULT_RUN_DEFAULTS,
  DEFAULT_SETTINGS,
  DEFAULT_WORKTREES,
  isRunDefaultsAtDefault,
  isThemeAtDefault,
  isWorktreesAtDefault,
} from "./settingsDefaults";
import { DEFAULT_RETENTION_HOURS } from "./retention";
import { DEFAULT_THEME_ID, THEMES } from "./themes";

describe("DEFAULT_SETTINGS", () => {
  // Guards the copy of `store::Settings::default` this module keeps: if the
  // backend defaults move, this test is the reminder to move them here too.
  it("matches the backend defaults", () => {
    expect(DEFAULT_SETTINGS).toEqual({
      default_agent: "claude",
      default_iterations: 1,
      concurrency_cap: 3,
      worktree_retention_hours: 48,
      cleanup_after_merge: true,
    });
    expect(DEFAULT_SETTINGS.worktree_retention_hours).toBe(DEFAULT_RETENTION_HOURS);
  });

  it("presents the retention default as the 'after N hours' mode", () => {
    expect(DEFAULT_WORKTREES).toEqual({
      mode: "after",
      hours: "48",
      cleanupAfterMerge: true,
    });
  });
});

describe("isRunDefaultsAtDefault", () => {
  it("is true for the defaults themselves", () => {
    expect(isRunDefaultsAtDefault(DEFAULT_RUN_DEFAULTS)).toBe(true);
  });

  it("is false when any one field differs", () => {
    expect(
      isRunDefaultsAtDefault({ ...DEFAULT_RUN_DEFAULTS, default_agent: "cursor" }),
    ).toBe(false);
    expect(
      isRunDefaultsAtDefault({ ...DEFAULT_RUN_DEFAULTS, default_iterations: 5 }),
    ).toBe(false);
    expect(
      isRunDefaultsAtDefault({ ...DEFAULT_RUN_DEFAULTS, concurrency_cap: 0 }),
    ).toBe(false);
  });
});

describe("isWorktreesAtDefault", () => {
  it("is true for the default mode and hour count", () => {
    expect(isWorktreesAtDefault(DEFAULT_WORKTREES)).toBe(true);
    expect(
      isWorktreesAtDefault({ mode: "after", hours: " 48 ", cleanupAfterMerge: true }),
    ).toBe(true);
  });

  it("is false for the other modes", () => {
    // Their stored encodings (0, -1) aren't the default 48 either way, but the
    // hour draft they carry alongside must not sway the answer.
    expect(
      isWorktreesAtDefault({ mode: "immediately", hours: "48", cleanupAfterMerge: true }),
    ).toBe(false);
    expect(
      isWorktreesAtDefault({ mode: "never", hours: "48", cleanupAfterMerge: true }),
    ).toBe(false);
  });

  it("is false for a different hour count", () => {
    expect(
      isWorktreesAtDefault({ mode: "after", hours: "12", cleanupAfterMerge: true }),
    ).toBe(false);
  });

  it("is false for a draft that only encodes to the default by fallback", () => {
    // These all save as 48, but none of them show 48 — Reset stays available
    // so the field can be put back in agreement with what would be saved.
    for (const hours of ["", "   ", "abc", "0", "-5"]) {
      expect(isWorktreesAtDefault({ mode: "after", hours, cleanupAfterMerge: true })).toBe(
        false,
      );
    }
  });

  it("is false when the cleanup toggle differs", () => {
    expect(
      isWorktreesAtDefault({ mode: "after", hours: "48", cleanupAfterMerge: false }),
    ).toBe(false);
  });
});

describe("isThemeAtDefault", () => {
  it("recognises the default theme", () => {
    expect(isThemeAtDefault(DEFAULT_THEME_ID)).toBe(true);
  });

  it("is false for every other registered theme", () => {
    for (const theme of THEMES.filter((t) => t.id !== DEFAULT_THEME_ID)) {
      expect(isThemeAtDefault(theme.id)).toBe(false);
    }
  });
});
