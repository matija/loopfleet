// Expansion state is the Story 16 "read the rest" affordance for truncated
// task rows: clicking the row's text toggles the key; a present key means
// expanded (the CSS wraps the full text), absence means collapsed. The map
// mutation rules below are what the hook and the row depend on — a regression
// here (e.g. flipping presence/absence, mutating in place) breaks expand.

import { describe, expect, it } from "vitest";
import { toggleKey } from "./taskExpand";

describe("toggleKey", () => {
  it("expands a collapsed key by adding it", () => {
    expect(toggleKey({}, "plan:1:task-a")).toEqual({ "plan:1:task-a": true });
  });

  it("collapses an expanded key by removing it", () => {
    expect(toggleKey({ "plan:1:task-a": true }, "plan:1:task-a")).toEqual({});
  });

  it("returns a new map and never mutates the input", () => {
    const input = { "plan:1:task-a": true };
    const next = toggleKey(input, "plan:1:task-b");
    expect(next).not.toBe(input);
    expect(input).toEqual({ "plan:1:task-a": true });
  });

  it("keeps sibling keys untouched when toggling one", () => {
    const input = { "plan:1:task-a": true };
    const next = toggleKey(input, "plan:2:task-z");
    expect(next).toEqual({
      "plan:1:task-a": true,
      "plan:2:task-z": true,
    });
  });

  it("is symmetric: two toggles return to the original state", () => {
    const input = { "plan:1:task-a": true };
    expect(toggleKey(toggleKey(input, "plan:1:task-a"), "plan:1:task-a")).toEqual(
      input,
    );
  });
});
