#!/usr/bin/env node
// Icon neutrality check: src-tauri/icons/icon.svg is deliberately accent-free
// (see the header comment in that file) because the app ships ten themes and
// none of them is built around a single hue baked into the app icon. This
// scans every hex colour in the SVG and fails if any channel spread (max
// minus min of R/G/B) exceeds 12/255 — i.e. the colour carries a visible hue
// rather than being a shade of grey.
//
// Intentional exceptions are allowed by adding a comment containing
// `icon-allow` either on the offending line itself or on the line
// immediately above it:
//
//   <stop offset="0" stop-color="#ff8800"/> <!-- icon-allow: launch badge -->

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const ICON_PATH = fileURLToPath(new URL("../../src-tauri/icons/icon.svg", import.meta.url));
const ALLOWLIST_MARKER = "icon-allow";
const MAX_CHANNEL_SPREAD = 12;

const HEX_COLOR_RE = /#[0-9a-fA-F]{3,8}\b/g;

function isAllowed(lines, index) {
  const line = lines[index];
  if (line.includes(ALLOWLIST_MARKER)) return true;
  const prev = lines[index - 1];
  return typeof prev === "string" && prev.includes(ALLOWLIST_MARKER);
}

function expandShortHex(hex) {
  if (hex.length === 3 || hex.length === 4) {
    return hex
      .split("")
      .map((c) => c + c)
      .join("");
  }
  return hex;
}

function channelSpread(hex) {
  const expanded = expandShortHex(hex);
  const r = parseInt(expanded.slice(0, 2), 16);
  const g = parseInt(expanded.slice(2, 4), 16);
  const b = parseInt(expanded.slice(4, 6), 16);
  return Math.max(r, g, b) - Math.min(r, g, b);
}

function findViolations(contents) {
  const lines = contents.split("\n");
  const violations = [];

  lines.forEach((line, i) => {
    if (isAllowed(lines, i)) return;

    for (const match of line.matchAll(HEX_COLOR_RE)) {
      const hex = match[0].slice(1);
      const spread = channelSpread(hex);
      if (spread > MAX_CHANNEL_SPREAD) {
        violations.push({ line: i + 1, value: match[0], spread });
      }
    }
  });

  return violations;
}

function main() {
  const contents = readFileSync(ICON_PATH, "utf8");
  const violations = findViolations(contents);

  if (violations.length === 0) {
    console.log("check-icon: icon.svg is neutral");
    return;
  }

  console.error(`check-icon: found ${violations.length} hued colour(s) in icon.svg:\n`);
  for (const v of violations) {
    console.error(`  icon.svg:${v.line}  ${v.value}  (channel spread ${v.spread} > ${MAX_CHANNEL_SPREAD})`);
  }
  console.error(
    `\nThe app icon must stay accent-free since no theme owns a hue. If this` +
      ` is an intentional one-off, add an "${ALLOWLIST_MARKER}" comment on the` +
      ` line (or the line above) explaining why.`,
  );
  process.exit(1);
}

main();
