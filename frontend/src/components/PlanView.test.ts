// Story 16 lock: truncation is only acceptable with a way to read the rest.
// The plan row's title clamps to the single --row-h line with an ellipsis
// (CSS), but the FULL text stays in the DOM and in the button's title
// attribute, and clicking the title toggles the expanded state that unwraps
// it — the row grows and the runs readout stays in its fixed metadata column.
// These tests read the component and its stylesheet so a future edit that
// clips the DOM text, drops the title, breaks the toggle wiring, or hides the
// runs fails here.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const PLAN_VIEW_SOURCE = readFileSync(
  fileURLToPath(new URL("./PlanView.tsx", import.meta.url)),
  "utf8",
);

const PLAN_CSS = readFileSync(
  fileURLToPath(new URL("../plan.css", import.meta.url)),
  "utf8",
);

/// The body of the CSS rule whose selector is `selector` followed by `{`
/// (unterminated — no closing brace), or null when no such rule exists.
/// Anchored on the opening brace so a selector that appears inside another
/// rule (e.g. the expanded-title selector inside `:has(...)`) is not
/// mistaken for its own rule.
function ruleBody(css: string, selector: string): string | null {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = css.match(new RegExp(escaped + "\\s*\\{"));
  if (!match || match.index === undefined) return null;
  const open = match.index + match[0].length - 1;
  return css.slice(open + 1, css.indexOf("}", open));
}

/// The row's truncated title button, as authored in the JSX.
function textButton(): string {
  const match = PLAN_VIEW_SOURCE.match(
    /<button\s+type="button"\s+className="task-row__text"[\s\S]*?<\/button>/,
  );
  expect(match, "task-row__text button must exist").not.toBeNull();
  return match![0];
}

describe("truncated task row: read-the-rest affordance", () => {
  it("renders the full text in the row, never a clipped summary", () => {
    const button = textButton();
    // The button's children are the full normalized task text — the same
    // expression the title attribute carries — not `taskSummary(...)`. A
    // truncation that clips in JS (dropping text from the DOM) fails here;
    // clipping must stay in CSS so expansion can unwrap the same text.
    expect(button).toContain("{normalizeDisplayText(task.text)}");
    expect(button).not.toContain("taskSummary(");
  });

  it("carries the full text in the title attribute", () => {
    expect(textButton()).toContain("title={normalizeDisplayText(task.text)}");
  });

  it("wires clicking the title to the persisted expanded state", () => {
    const button = textButton();
    expect(button).toContain('aria-expanded={expanded}');
    expect(button).toContain("onClick={toggleExpanded}");
  });

  it("keys the expansion by plan id + task anchor, not list index", () => {
    expect(PLAN_VIEW_SOURCE).toContain(
      "useTaskExpanded(`${planId}:${task.anchor}`)",
    );
  });

  it("clamps the title to one line at rest (CSS truncation)", () => {
    const body = ruleBody(PLAN_CSS, ".task-row__text") ?? "";
    expect(body).toContain("white-space: nowrap");
    expect(body).toContain("overflow: hidden");
    expect(body).toContain("text-overflow: ellipsis");
  });

  it("expanded state unwraps the full text instead of clipping it", () => {
    const body = ruleBody(PLAN_CSS, '.task-row__text[aria-expanded="true"]') ?? "";
    expect(body).toContain("white-space: normal");
    expect(body).toContain("overflow: visible");
    expect(body).toContain("text-overflow: clip");
  });

  it("lets the row grow to fit the wrapped text when expanded", () => {
    const body =
      ruleBody(
        PLAN_CSS,
        '.task-row:has(.task-row__text[aria-expanded="true"])',
      ) ?? "";
    expect(body).toContain("height: auto");
    expect(body).toContain("min-height");
  });

  it("keeps the runs readout outside the truncating element", () => {
    // The run count and compare entry live in the fixed metadata column, a
    // sibling of the clipped title — never inside it, so truncation and
    // expansion cannot hide them.
    const metaMatch = PLAN_VIEW_SOURCE.match(
      /<span className="task-row__meta">[\s\S]*?<\/span>\n(\s*)(?=<LaunchControl|\{lastRun)/,
    );
    expect(metaMatch, "task-row__meta column must exist").not.toBeNull();
    const meta = metaMatch![0];
    expect(meta).toContain("task-row__compare");
    expect(meta).toContain("task-row__run-count");
    // The compare target carries the full authored text, so opening the runs
    // from a truncated row still knows the whole task.
    expect(meta).toContain("taskText: task.text");

    // And the CSS keeps that column out of the row's flex grow/shrink.
    const metaCss = ruleBody(PLAN_CSS, ".task-row__meta") ?? "";
    expect(metaCss).toContain("flex: none");
    expect(metaCss).not.toMatch(/display:\s*none/);
    expect(metaCss).not.toMatch(/visibility:\s*hidden/);
  });
});
