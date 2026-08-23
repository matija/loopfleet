import { describe, expect, it } from "vitest";
import {
  DEFAULT_RETENTION_HOURS,
  retentionModeOf,
  retentionValue,
} from "./retention";

describe("retentionModeOf", () => {
  it("maps the three stored encodings to their modes", () => {
    expect(retentionModeOf(0)).toBe("immediately");
    expect(retentionModeOf(48)).toBe("after");
    expect(retentionModeOf(-1)).toBe("never");
  });

  it("treats any negative value as never", () => {
    expect(retentionModeOf(-999)).toBe("never");
  });
});

describe("retentionValue", () => {
  it("encodes each mode", () => {
    expect(retentionValue("immediately", "48")).toBe(0);
    expect(retentionValue("never", "48")).toBe(-1);
    expect(retentionValue("after", "12")).toBe(12);
  });

  it("ignores the hour draft for the two fixed modes", () => {
    expect(retentionValue("immediately", "")).toBe(0);
    expect(retentionValue("never", "abc")).toBe(-1);
  });

  it("falls back to the default rather than 0 for an unusable draft", () => {
    // 0 would mean "reap immediately" — never what a half-typed field means.
    for (const draft of ["", "   ", "abc", "0", "-5", "0.5"]) {
      expect(retentionValue("after", draft)).toBe(DEFAULT_RETENTION_HOURS);
    }
  });

  it("floors fractional hours", () => {
    expect(retentionValue("after", "1.9")).toBe(1);
  });
});
