import { describe, expect, it } from "vitest";
import { formatDuration } from "./DataGrid";

describe("formatDuration", () => {
  it("renders sub-second spans in milliseconds", () => {
    expect(formatDuration(0)).toBe("0ms");
    expect(formatDuration(42)).toBe("42ms");
    expect(formatDuration(999)).toBe("999ms");
  });

  it("renders sub-minute spans in seconds to one decimal", () => {
    expect(formatDuration(1000)).toBe("1.0s");
    expect(formatDuration(1300)).toBe("1.3s");
    expect(formatDuration(59999)).toBe("60.0s");
  });

  it("renders sub-hour spans as minutes and seconds", () => {
    expect(formatDuration(60_000)).toBe("1m 00s");
    expect(formatDuration(125_000)).toBe("2m 05s");
  });

  it("renders hour-plus spans as hours and minutes", () => {
    expect(formatDuration(3_600_000)).toBe("1h 00m");
    expect(formatDuration(3_600_000 + 4 * 60_000)).toBe("1h 04m");
  });

  it("returns the placeholder for a missing or unusable span", () => {
    expect(formatDuration(null)).toBe("—");
    expect(formatDuration(-1)).toBe("—");
    expect(formatDuration(Infinity)).toBe("—");
    expect(formatDuration(NaN)).toBe("—");
  });
});
