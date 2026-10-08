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

  it("shows native automation even without write overrides", () => {
    expect(overrideSummary([], true, true)).toBe("native automation");
    expect(overrideSummary(["/tmp/cache"], true, true)).toBe("1 path · native automation");
    expect(overrideSummary([], false, true)).toBe("");
  });

  it("counts, singular and plural", () => {
    expect(overrideSummary(["/tmp/cache"], true)).toBe("1 path");
    expect(overrideSummary(["/tmp/cache", "/tmp/build"], true)).toBe("2 paths");
  });
});
