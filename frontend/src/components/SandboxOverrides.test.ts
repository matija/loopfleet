import { describe, expect, it } from "vitest";
import { overrideSummary } from "./SandboxOverrides";

describe("overrideSummary", () => {
  it("says nothing until the saved list has loaded", () => {
    expect(overrideSummary([], false)).toBe("");
    expect(overrideSummary(["/tmp/cache"], false)).toBe("");
  });

  it("reports an empty list as 'none' once loaded", () => {
    expect(overrideSummary([], true)).toBe("none");
  });

  it("counts, singular and plural", () => {
    expect(overrideSummary(["/tmp/cache"], true)).toBe("1 path");
    expect(overrideSummary(["/tmp/cache", "/tmp/build"], true)).toBe("2 paths");
  });
});
