import { describe, expect, it } from "vitest";
import { validArchiveName } from "./archiveName";

describe("validArchiveName", () => {
  it("accepts a well-formed name", () => {
    expect(validArchiveName("my-plan-123.md")).toBeNull();
    expect(validArchiveName("a.md")).toBeNull();
  });

  it("rejects an empty name", () => {
    expect(validArchiveName("")).toBe("name is empty");
  });

  it("rejects path separators", () => {
    expect(validArchiveName("a/b.md")).toBe("name contains a path separator");
    expect(validArchiveName("a\\b.md")).toBe("name contains a path separator");
  });

  it("rejects '..'", () => {
    expect(validArchiveName("a..b.md")).toBe('name contains ".."');
  });

  it("requires a .md suffix", () => {
    expect(validArchiveName("plan")).toBe('name must end in ".md"');
    expect(validArchiveName("plan.txt")).toBe('name must end in ".md"');
  });

  it("rejects a stem that is empty once the suffix is stripped", () => {
    expect(validArchiveName(".md")).toBe("name is empty");
  });

  it("rejects a stem starting or ending with a hyphen", () => {
    expect(validArchiveName("-plan.md")).toBe("name starts with a hyphen");
    expect(validArchiveName("plan-.md")).toBe("name ends with a hyphen");
  });

  it("rejects consecutive hyphens", () => {
    expect(validArchiveName("my--plan.md")).toBe(
      "name contains consecutive hyphens",
    );
  });

  it("rejects uppercase and other invalid characters", () => {
    expect(validArchiveName("Plan.md")).toBe(
      "name contains an invalid character: 'P'",
    );
    expect(validArchiveName("plan_1.md")).toBe(
      "name contains an invalid character: '_'",
    );
    expect(validArchiveName("plan 1.md")).toBe(
      "name contains an invalid character: ' '",
    );
  });
});
