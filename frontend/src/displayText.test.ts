import { describe, expect, it } from "vitest";
import { taskSummary } from "./displayText";

describe("taskSummary", () => {
  it("returns short text unchanged", () => {
    expect(taskSummary("Fix the build")).toBe("Fix the build");
  });

  it("keeps a single sentence with trailing period unchanged", () => {
    expect(taskSummary("Fix the build.")).toBe("Fix the build.");
  });

  it("cuts at the first sentence boundary with an ellipsis", () => {
    expect(taskSummary("Wire up the dock. Then polish the styling.")).toBe(
      "Wire up the dock…",
    );
  });

  it("treats ! and ? as sentence boundaries", () => {
    expect(taskSummary("Ship it! More work follows.")).toBe("Ship it…");
    expect(taskSummary("Does it build? Check CI first.")).toBe("Does it build…");
  });

  it("cuts at a word boundary within 140 chars when there is no sentence boundary", () => {
    const words = Array(40).fill("component").join(" "); // 399 chars, no punctuation
    const summary = taskSummary(words);
    expect(summary.endsWith("…")).toBe(true);
    const text = summary.slice(0, -1);
    expect(text.length).toBeLessThanOrEqual(140);
    // Never mid-word: the clipped text must align with a word boundary.
    expect(words.startsWith(text)).toBe(true);
    expect(words[text.length]).toBe(" ");
  });

  it("uses the 140-char cut when the first sentence boundary lies beyond it", () => {
    const long = "word ".repeat(35).trim() + ". Second sentence."; // boundary at 175
    const summary = taskSummary(long);
    expect(summary.endsWith("…")).toBe(true);
    expect(summary.length).toBeLessThanOrEqual(141);
  });

  it("hard-cuts a single unbroken run longer than 140 chars", () => {
    const run = "x".repeat(200);
    expect(taskSummary(run)).toBe("x".repeat(140) + "…");
  });

  it("strips markdown before measuring", () => {
    expect(taskSummary("**Fix** the `build` now")).toBe("Fix the build now");
  });

  it("does not count markdown syntax toward the limit", () => {
    // 138 visible chars once markdown is stripped, but >140 raw.
    const visible = "a".repeat(134) + " end";
    const marked = "**" + "a".repeat(134) + "** `end`";
    expect(taskSummary(marked)).toBe(visible);
  });

  it("cuts markdown-heavy text on the stripped form", () => {
    const marked = "`Update` the **dock**. Then refactor.";
    expect(taskSummary(marked)).toBe("Update the dock…");
  });
});
