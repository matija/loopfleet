import { describe, expect, it } from "vitest";
import { clampToEnabled, enabledIndices, moveIndex } from "./optionIndex";

const abc = [{}, {}, {}];
const middleDisabled = [{}, { disabled: true }, {}];

describe("enabledIndices", () => {
  it("keeps every option's original position", () => {
    expect(enabledIndices(abc)).toEqual([0, 1, 2]);
    expect(enabledIndices(middleDisabled)).toEqual([0, 2]);
  });

  it("is empty when nothing can be picked", () => {
    expect(enabledIndices([])).toEqual([]);
    expect(enabledIndices([{ disabled: true }, { disabled: true }])).toEqual([]);
  });
});

describe("clampToEnabled", () => {
  const indices = [1, 3, 4];

  it("leaves an already-enabled index alone", () => {
    expect(clampToEnabled(indices, 3)).toBe(3);
  });

  it("lands on the nearest enabled option after a disabled one", () => {
    expect(clampToEnabled(indices, 0)).toBe(1);
    expect(clampToEnabled(indices, 2)).toBe(3);
  });

  it("wraps to the first enabled option when none follow", () => {
    expect(clampToEnabled(indices, 9)).toBe(1);
  });

  it("has nowhere better to go when nothing is enabled", () => {
    expect(clampToEnabled([], 2)).toBe(2);
  });
});

describe("moveIndex", () => {
  const indices = [0, 2, 3];

  it("skips disabled options in both directions", () => {
    expect(moveIndex(indices, 0, 1, "clamp")).toBe(2);
    expect(moveIndex(indices, 2, -1, "clamp")).toBe(0);
  });

  it("stops at the ends in clamp mode", () => {
    expect(moveIndex(indices, 0, -1, "clamp")).toBe(0);
    expect(moveIndex(indices, 3, 1, "clamp")).toBe(3);
  });

  it("goes around the ends in wrap mode", () => {
    expect(moveIndex(indices, 3, 1, "wrap")).toBe(0);
    expect(moveIndex(indices, 0, -1, "wrap")).toBe(3);
  });

  it("wraps across more than one full lap", () => {
    // Three enabled options, so ±4 is one full lap plus one step.
    expect(moveIndex(indices, 0, 4, "wrap")).toBe(2);
    expect(moveIndex(indices, 0, -4, "wrap")).toBe(3);
  });

  it("honours a delta larger than one step", () => {
    expect(moveIndex(indices, 0, 2, "clamp")).toBe(3);
    expect(moveIndex(indices, 0, 0, "clamp")).toBe(0);
  });

  it("treats a disabled or unknown origin as the first enabled option", () => {
    // Index 1 is disabled, so it never sits in `indices`.
    expect(moveIndex(indices, 1, 1, "clamp")).toBe(2);
    expect(moveIndex(indices, 1, -1, "clamp")).toBe(0);
    expect(moveIndex(indices, -1, 1, "wrap")).toBe(2);
  });

  it("stays put when there is nowhere to move", () => {
    expect(moveIndex([], 2, 1, "clamp")).toBe(2);
    expect(moveIndex([], 2, -1, "wrap")).toBe(2);
  });

  it("keeps a single enabled option under any delta", () => {
    expect(moveIndex([2], 2, 1, "wrap")).toBe(2);
    expect(moveIndex([2], 2, -3, "clamp")).toBe(2);
  });
});
