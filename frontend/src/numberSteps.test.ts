import { describe, expect, it } from "vitest";
import {
  atMax,
  atMin,
  clampToRange,
  normalizeValue,
  parseFieldValue,
  roundToStep,
  stepValue,
} from "./numberSteps";

describe("clampToRange", () => {
  it("pulls values inside both bounds", () => {
    expect(clampToRange(5, 1, 3)).toBe(3);
    expect(clampToRange(0, 1, 3)).toBe(1);
    expect(clampToRange(2, 1, 3)).toBe(2);
  });

  it("leaves an absent bound open", () => {
    expect(clampToRange(-100, undefined, 3)).toBe(-100);
    expect(clampToRange(100, 1, undefined)).toBe(100);
    expect(clampToRange(42)).toBe(42);
  });

  it("treats zero as a real bound, not a missing one", () => {
    expect(clampToRange(-5, 0)).toBe(0);
    expect(clampToRange(5, undefined, 0)).toBe(0);
  });
});

describe("roundToStep", () => {
  it("keeps whole numbers whole for an integer step", () => {
    expect(roundToStep(3.7, 1)).toBe(4);
    expect(roundToStep(3.2, 1)).toBe(3);
  });

  it("rounds to the step's own decimal precision", () => {
    expect(roundToStep(0.30000000000000004, 0.1)).toBe(0.3);
    expect(roundToStep(1.0049, 0.01)).toBe(1);
    // Precision comes from the step's digits (0.25 has two), not its size.
    expect(roundToStep(1.007, 0.25)).toBe(1.01);
  });

  it("rounds to the digits a step declares, not to a multiple of the step", () => {
    // 1.2 is not a multiple of 0.5; the step only sets the decimal precision,
    // which is all the field needs to stop drift accumulating.
    expect(roundToStep(1.24, 0.5)).toBe(1.2);
  });
});

describe("normalizeValue", () => {
  const bounds = { min: 0, max: 10, step: 1 };

  it("rounds and then clamps", () => {
    expect(normalizeValue(4.6, bounds)).toBe(5);
    expect(normalizeValue(99, bounds)).toBe(10);
    expect(normalizeValue(-3, bounds)).toBe(0);
  });

  it("clamps after rounding, so a rounded-up value still respects max", () => {
    expect(normalizeValue(9.7, bounds)).toBe(10);
  });
});

describe("stepValue", () => {
  it("moves one step per press", () => {
    expect(stepValue(3, 1, { step: 1 })).toBe(4);
    expect(stepValue(3, -1, { step: 1 })).toBe(2);
  });

  it("stops at the bounds instead of overshooting", () => {
    expect(stepValue(10, 1, { min: 0, max: 10, step: 1 })).toBe(10);
    expect(stepValue(0, -1, { min: 0, max: 10, step: 1 })).toBe(0);
  });

  it("pulls an out-of-range starting value back in", () => {
    expect(stepValue(50, 1, { min: 0, max: 10, step: 1 })).toBe(10);
  });

  it("does not accumulate floating-point drift", () => {
    const bounds = { min: 0, max: 1, step: 0.1 };
    let v = 0;
    for (let i = 0; i < 10; i++) v = stepValue(v, 0.1, bounds);
    expect(v).toBe(1);
    // The naive version of this loop lands on 0.9999999999999999.
    expect(stepValue(0.2, 0.1, bounds)).toBe(0.3);
  });

  it("honours a fractional step's precision on the way down too", () => {
    expect(stepValue(0.3, -0.1, { step: 0.1 })).toBe(0.2);
  });
});

describe("parseFieldValue", () => {
  it("reads a typed number", () => {
    expect(parseFieldValue("42")).toBe(42);
    expect(parseFieldValue("-1.5")).toBe(-1.5);
    expect(parseFieldValue(" 7 ")).toBe(7);
  });

  it("reads blank input as no number rather than 0", () => {
    // Number("") is 0, which would silently reset a cleared field to zero.
    expect(parseFieldValue("")).toBeNull();
    expect(parseFieldValue("   ")).toBeNull();
  });

  it("rejects half-typed and non-numeric input", () => {
    expect(parseFieldValue("-")).toBeNull();
    expect(parseFieldValue("1e")).toBeNull();
    expect(parseFieldValue("abc")).toBeNull();
    expect(parseFieldValue("Infinity")).toBeNull();
  });
});

describe("atMin / atMax", () => {
  it("reports a value sitting on a bound", () => {
    expect(atMin(0, 0)).toBe(true);
    expect(atMax(10, 10)).toBe(true);
  });

  it("reports a value past a bound, so the stepper stays disabled", () => {
    expect(atMin(-1, 0)).toBe(true);
    expect(atMax(11, 10)).toBe(true);
  });

  it("is false in the middle of the range", () => {
    expect(atMin(5, 0)).toBe(false);
    expect(atMax(5, 10)).toBe(false);
  });

  it("never reaches an absent bound", () => {
    expect(atMin(-999)).toBe(false);
    expect(atMax(999)).toBe(false);
  });
});
