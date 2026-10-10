import { describe, expect, it } from "vitest";
import { isValidTapMessage } from "./TapComposer";

describe("isValidTapMessage", () => {
  it.each(["", " ", "\t\r\n "])("rejects blank input %j", (text) => {
    expect(isValidTapMessage(text)).toBe(false);
  });

  it("accepts valid multiline input", () => {
    expect(isValidTapMessage("  First line\n\nSecond line\n  ")).toBe(true);
  });
});
