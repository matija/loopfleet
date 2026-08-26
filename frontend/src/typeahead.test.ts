import { describe, expect, it } from "vitest";
import {
  TYPEAHEAD_RESET_MS,
  emptyTypeahead,
  extendTypeahead,
  matchTypeahead,
} from "./typeahead";

describe("extendTypeahead", () => {
  it("starts a search from the first keystroke", () => {
    expect(extendTypeahead(emptyTypeahead, "s", 5_000)).toEqual({ text: "s", at: 5_000 });
  });

  it("appends keystrokes typed inside the reset window", () => {
    const first = extendTypeahead(emptyTypeahead, "s", 1_000);
    const second = extendTypeahead(first, "e", 1_400);
    expect(second).toEqual({ text: "se", at: 1_400 });
    expect(extendTypeahead(second, "t", 1_500).text).toBe("set");
  });

  it("starts over after a pause of the reset window or longer", () => {
    const first = extendTypeahead(emptyTypeahead, "s", 1_000);
    const after = extendTypeahead(first, "e", 1_000 + TYPEAHEAD_RESET_MS);
    expect(after).toEqual({ text: "e", at: 1_000 + TYPEAHEAD_RESET_MS });
  });

  it("keeps extending as long as each gap stays under the window", () => {
    // The window is per-keystroke, not per-search: a slow but steady typist
    // keeps one search alive well past TYPEAHEAD_RESET_MS in total.
    let state = extendTypeahead(emptyTypeahead, "a", 0);
    state = extendTypeahead(state, "b", 900);
    state = extendTypeahead(state, "c", 1_800);
    expect(state.text).toBe("abc");
  });

  it("does not treat the empty buffer as a live search", () => {
    // A first keypress at t=0 must not look like a continuation of `at: 0`.
    expect(extendTypeahead(emptyTypeahead, "x", 0).text).toBe("x");
  });
});

describe("matchTypeahead", () => {
  const labels = ["Auto", "Always", "Never", "Ask"];
  const all = [0, 1, 2, 3];

  it("finds the first label with the prefix", () => {
    expect(matchTypeahead(labels, all, 0, "n")).toBe(2);
  });

  it("ignores case on both sides", () => {
    expect(matchTypeahead(["settings"], [0], -1, "SET")).toBe(0);
    expect(matchTypeahead(labels, all, -1, "aU")).toBe(0);
  });

  it("cycles through same-initial options on repeated presses", () => {
    // "a" from the top lands on Auto, then Always, then Ask, then back round.
    expect(matchTypeahead(labels, all, -1, "a")).toBe(0);
    expect(matchTypeahead(labels, all, 0, "a")).toBe(1);
    expect(matchTypeahead(labels, all, 1, "a")).toBe(3);
    expect(matchTypeahead(labels, all, 3, "a")).toBe(0);
  });

  it("can match the option already highlighted, but only last", () => {
    expect(matchTypeahead(labels, all, 2, "ne")).toBe(2);
  });

  it("narrows to a longer search rather than cycling", () => {
    expect(matchTypeahead(labels, all, 0, "al")).toBe(1);
  });

  it("skips disabled options", () => {
    // Always (1) is disabled, so "a" from Auto continues to Ask.
    expect(matchTypeahead(labels, [0, 2, 3], 0, "a")).toBe(3);
  });

  it("returns undefined when nothing matches", () => {
    expect(matchTypeahead(labels, all, 0, "z")).toBeUndefined();
    expect(matchTypeahead(labels, [], 0, "a")).toBeUndefined();
  });

  it("matches nothing on an empty search", () => {
    // Every label starts with "", which would otherwise move the highlight on
    // a keystroke that contributed no text.
    expect(matchTypeahead(labels, all, 0, "")).toBeUndefined();
  });
});
