#!/usr/bin/env node
// Density check: the two structural rules the density pass relies on staying
// fixed (PRD Phase 4 / Story 24), scanned the way check-native-controls.mjs
// scans sources — list files, one regex per line, collect errors, exit 1 with
// a message naming the file and line.
//
// (a) A `max-width` in `ch` units is a reading measure: right for prose and
//     wrong for a list of rows. It is allowed only on the prose surfaces
//     named in PROSE_SURFACES below, so a 72ch gutter reintroduced on a
//     non-prose surface fails instead of quietly insetting a pane.
//
// (b) A rule that combines a box `border` declaration (anything but `none` or
//     `0`) with a `padding` at or above --space-4 is a padded card — the
//     shape DESIGN.md rejects ("don't create gratuitous cards or nest padded
//     containers") and the reference never shows. Surfaces structure with
//     hairlines; a rule that grows a bordered, padded box fails.
//
// Deliberate exceptions use the same tokens-allow-style inline escape hatch
// check-tokens.mjs uses, under this file's own marker `density-allow`: a
// comment containing the marker on the offending line itself or on the line
// immediately above it:
//
//   padding: var(--space-5); /* density-allow: reason */
//
// The three card surfaces that deliberately keep their frame (.empty-state,
// .prd-doc, .surface-card) carry such comments on their padding lines (or on
// the line directly above).

import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join, relative } from "node:path";

const SRC_DIR = fileURLToPath(new URL("../src/", import.meta.url));
const ALLOWLIST_MARKER = "density-allow";

// The prose surfaces: the only places a ch-capped width is a reading measure.
// Every current ch gutter in the tree belongs to one of these; a ch gutter
// anywhere else is a re-introduced prose gutter and fails.
const PROSE_SURFACES = new Set([
  ".prd-doc__body", // running PRD prose in its measured column (72ch)
  ".empty-state", // no-content note card, centered at a reading measure (52ch)
  ".empty-prompt__title", // no-project prompt heading (44ch)
  ".empty-prompt__subtitle", // no-project prompt copy (48ch)
  ".hint__detail", // expanded hint prose — "a few sentences break naturally" (42ch)
  ".launch__result", // launch-result popover message cap (24ch)
  ".launch__blocked", // blocked-launch prompt message cap (30ch)
  ".main__placeholder", // pane placeholder paragraph (60ch)
]);

const CH_MAX_WIDTH_RE = /\bmax-width\s*:\s*([0-9]+(?:\.[0-9]+)?)ch\b/;

// A full `border:` shorthand frames a box; `none`/`0` borders are bare. The
// value is read before the trailing `;` so "border: none;" isn't a frame.
const BOX_BORDER_RE = /^\s*border\s*:\s*(.+)$/;

// Splits a padding value into components, keeping calc()/var() intact.
function splitValue(value) {
  const parts = [];
  let current = "";
  let depth = 0;
  for (const char of value) {
    if (char === "(") depth++;
    if (char === ")") depth--;
    if (/\s/.test(char) && depth === 0) {
      if (current) {
        parts.push(current);
        current = "";
      }
    } else {
      current += char;
    }
  }
  if (current) parts.push(current);
  return parts;
}

// A component is "at or above --space-4" when it uses the panel-padding band
// token (--space-4/5/6/8) or a raw px value that large.
function inPaddingBand(component) {
  const value = component.trim();
  if (/^var\(--space-[4-9](?:\)|$)|^var\(--space-[1-9][0-9](?:\)|$)/.test(value)) return true;
  const px = value.match(/^([0-9]+(?:\.[0-9]+)?)px$/);
  return px !== null && Number(px[1]) >= 14; // --space-4 is 14px
}

// The card shape comes from block (top/bottom) padding, not the horizontal
// inset a control carries: `padding: var(--space-1) var(--space-4)` is a
// button, `padding: var(--space-4)` is a padded box. Only the full shorthand
// or the block sides count; padding-left/right never does.
function hasBlockBandPadding(declaration) {
  const match = declaration.match(/^\s*padding(?:-top|-bottom)?\s*:\s*(.+)$/);
  if (!match) return false;
  const value = match[1].trim().replace(/;$/, "");
  const parts = splitValue(value);
  if (parts.length === 1) return inPaddingBand(parts[0]);
  if (parts.length === 2) return inPaddingBand(parts[0]); // block = first
  if (parts.length === 3) return inPaddingBand(parts[0]) || inPaddingBand(parts[2]);
  return inPaddingBand(parts[0]) || inPaddingBand(parts[2]);
}

function hasBoxBorder(declaration) {
  const match = declaration.match(BOX_BORDER_RE);
  if (!match) return false;
  const value = match[1].trim().replace(/;$/, "");
  return !/^(?:none|0(?:px|em|rem)?)$/.test(value);
}

// A rule's selector matches an allowlist entry when one of its comma-separated
// parts is that selector (or carries it plus a pseudo-class/state).
function selectorMatches(selector, allowed) {
  return selector
    .split(",")
    .map((part) => part.trim())
    .some(
      (part) => part === allowed || part.startsWith(`${allowed}:`),
    );
}

function isProseSurface(selector) {
  for (const surface of PROSE_SURFACES) {
    if (selectorMatches(selector, surface)) return true;
  }
  return false;
}

function isAllowed(rawLines, index) {
  const line = rawLines[index];
  if (line.includes(ALLOWLIST_MARKER)) return true;
  const prev = rawLines[index - 1];
  return typeof prev === "string" && prev.includes(ALLOWLIST_MARKER);
}

// Comment stripping preserves line count (comments become spaces, newlines
// stay), so reported line numbers keep pointing at the source.
function stripComments(text) {
  return text.replace(/\/\*[\s\S]*?\*\//g, (comment) =>
    comment.replace(/[^\n]/g, " "),
  );
}

function listCssFiles(dir) {
  const files = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) files.push(...listCssFiles(path));
    else if (entry.isFile() && entry.name.endsWith(".css")) files.push(path);
  }
  return files;
}

const errors = [];

function checkFile(path) {
  const rel = `src/${relative(SRC_DIR, path)}`;
  const raw = readFileSync(path, "utf8");
  const rawLines = raw.split("\n");
  const lines = stripComments(raw).split("\n");

  // Walk rules by brace depth so each declaration can be attributed to the
  // rule (and selector) it lives in. Only leaf rules carry declarations.
  const stack = []; // { selector, body: [{ line, text }], nested }
  let prelude = "";

  const push = (selector) => {
    stack.push({ selector, body: [], nested: false });
    if (stack.length > 1) stack[stack.length - 2].nested = true;
  };

  for (let i = 0; i < lines.length; i++) {
    const code = lines[i];

    if (code.includes("{")) {
      const at = code.indexOf("{");
      prelude = `${prelude} ${code.slice(0, at)}`.replace(/\s+/g, " ").trim();
      push(prelude);
      prelude = "";
      const rest = code.slice(at + 1).trim();
      if (rest) stack[stack.length - 1].body.push({ line: i + 1, text: rest });
    } else if (code.includes("}")) {
      const at = code.indexOf("}");
      const before = code.slice(0, at).trim();
      if (before && stack.length) {
        stack[stack.length - 1].body.push({ line: i + 1, text: before });
      }
      const closed = stack.pop();
      if (closed && !closed.nested) inspectRule(rel, closed, rawLines);
      prelude = "";
      const rest = code.slice(at + 1).trim();
      if (rest) prelude = rest;
    } else if (stack.length) {
      stack[stack.length - 1].body.push({ line: i + 1, text: code });
    } else if (code.trim()) {
      prelude = `${prelude} ${code}`.replace(/\s+/g, " ").trim();
    }
  }
}

function inspectRule(rel, rule, rawLines) {
  let borderLine = null;
  let paddingLine = null;
  let paddingText = "";

  for (const declaration of rule.body) {
    const text = declaration.text.trim();
    if (!text) continue;

    if (borderLine === null && hasBoxBorder(text)) {
      borderLine = declaration.line;
    }
    if (paddingLine === null && hasBlockBandPadding(text)) {
      paddingLine = declaration.line;
      paddingText = text;
    }
  }

  if (borderLine !== null && paddingLine !== null) {
    const index = paddingLine - 1;
    if (!isAllowed(rawLines, index)) {
      errors.push(
        `${rel}:${paddingLine}: rule "${rule.selector}" pairs a box border` +
          ` (line ${borderLine}) with ${paddingText} — the padded-card shape,` +
          ` a border with padding at or above --space-4. Structure the surface` +
          ` with hairlines instead; if the card is deliberate, put a` +
          ` "${ALLOWLIST_MARKER}" comment on the padding line (or the line` +
          ` above):\n      ${paddingText}`,
      );
    }
  }

  for (const declaration of rule.body) {
    const text = declaration.text.trim();
    if (!text) continue;
    const match = text.match(CH_MAX_WIDTH_RE);
    if (!match) continue;
    if (isProseSurface(rule.selector)) continue;
    if (isAllowed(rawLines, declaration.line - 1)) continue;
    errors.push(
      `${rel}:${declaration.line}: ${match[1]}ch max-width on` +
        ` "${rule.selector}" — a ch measure is a reading measure for prose,` +
        ` and this rule is not one of the allowlisted prose surfaces:\n` +
        `      ${text}`,
    );
  }
}

function main() {
  const cssFiles = listCssFiles(SRC_DIR)
    .filter((path) => relative(SRC_DIR, path) !== "tokens.css")
    .sort();
  for (const path of cssFiles) checkFile(path);

  if (errors.length === 0) {
    console.log(`check-density: ${cssFiles.length} files clean`);
    return;
  }

  console.error(`check-density: found ${errors.length} density violation(s):\n`);
  for (const error of errors) console.error(`  ${error}`);
  console.error(
    `\nA max-width in ch units is a prose measure, and a rule that combines a` +
      ` box border with --space-4+ padding is a padded card — the density pass` +
      ` removes both. If this case is deliberate, add a "${ALLOWLIST_MARKER}"` +
      ` comment on the line (or the line above) explaining why, exactly like` +
      ` check-tokens.mjs's "tokens-allow" hatch.`,
  );
  process.exit(1);
}

main();
