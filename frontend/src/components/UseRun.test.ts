import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const USE_RUN_SOURCE = readFileSync(
  fileURLToPath(new URL("./UseRun.tsx", import.meta.url)),
  "utf8",
);

describe("use-run merged marker", () => {
  it("stays a non-interactive span with its accessible name intact", () => {
    // Mirrors RunDock's run-chip__merged lock: once a run lands, the shared
    // "use this run" control (compare, timeline) must swap to a static glyph
    // rather than leaving a live merge control on an already-merged run.
    const markerMatch = USE_RUN_SOURCE.match(
      /<span\s+className="use-run__merged"[\s\S]*?<\/span>/,
    );
    expect(markerMatch).not.toBeNull();
    const marker = markerMatch![0];

    expect(marker).toContain('role="img"');
    expect(marker).toContain('aria-label="Merged"');
    expect(marker).not.toMatch(/onClick/);
    expect(marker).not.toMatch(/tabIndex/);
    expect(marker).not.toMatch(/role="button"/);
  });
});
