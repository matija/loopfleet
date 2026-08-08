#!/usr/bin/env node
// Token-discipline check: every frontend/src/*.css file (besides tokens.css,
// which *defines* the raw values) must reference design tokens rather than
// hardcoding a color or a radius/height in px. This keeps --c-*, --radius-*,
// --space-*, etc. as the single source of truth for the design system.
//
// Intentional exceptions (a handful of decorative one-offs that don't belong
// in the token scale, e.g. a 6px status dot) are allowed by adding a comment
// containing `tokens-allow` either on the offending line itself or on the
// line immediately above it:
//
//   width: 6px; /* tokens-allow: decorative status dot */
//
//   /* tokens-allow: decorative status dot */
//   height: 6px;

import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";

const SRC_DIR = fileURLToPath(new URL("../src/", import.meta.url));
const ALLOWLIST_MARKER = "tokens-allow";

const HEX_COLOR_RE = /#[0-9a-fA-F]{3,8}\b/g;
const RGB_FUNC_RE = /\brgba?\(/g;
const PX_RADIUS_HEIGHT_RE =
  /\b(?:border-radius|border-(?:top|bottom)-(?:left|right)-radius|height|min-height|max-height)\s*:\s*[0-9.]+px/g;

function isAllowed(lines, index) {
  const line = lines[index];
  if (line.includes(ALLOWLIST_MARKER)) return true;
  const prev = lines[index - 1];
  return typeof prev === "string" && prev.includes(ALLOWLIST_MARKER);
}

function findViolations(file, contents) {
  const lines = contents.split("\n");
  const violations = [];

  lines.forEach((line, i) => {
    if (isAllowed(lines, i)) return;

    for (const match of line.matchAll(HEX_COLOR_RE)) {
      violations.push({ file, line: i + 1, kind: "hex color", value: match[0] });
    }
    for (const match of line.matchAll(RGB_FUNC_RE)) {
      violations.push({ file, line: i + 1, kind: "rgb()/rgba() literal", value: match[0] });
    }
    for (const match of line.matchAll(PX_RADIUS_HEIGHT_RE)) {
      violations.push({ file, line: i + 1, kind: "bare px radius/height", value: match[0].trim() });
    }
  });

  return violations;
}

function main() {
  const cssFiles = readdirSync(SRC_DIR)
    .filter((name) => name.endsWith(".css") && name !== "tokens.css")
    .sort();

  const violations = cssFiles.flatMap((name) => {
    const path = join(SRC_DIR, name);
    const contents = readFileSync(path, "utf8");
    return findViolations(`src/${name}`, contents);
  });

  if (violations.length === 0) {
    console.log(`check-tokens: ${cssFiles.length} files clean`);
    return;
  }

  console.error(`check-tokens: found ${violations.length} token violation(s):\n`);
  for (const v of violations) {
    console.error(`  ${v.file}:${v.line}  ${v.kind}  ->  ${v.value}`);
  }
  console.error(
    `\nUse design tokens from src/tokens.css instead of raw values. If this is` +
      ` an intentional one-off, add a "${ALLOWLIST_MARKER}" comment on the` +
      ` line (or the line above) explaining why.`,
  );
  process.exit(1);
}

main();
