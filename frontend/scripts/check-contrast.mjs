#!/usr/bin/env node
// Contrast check: computes WCAG 2.2 relative-luminance contrast ratios for
// every foreground/background pair the app actually uses, and fails the
// build if any pair falls short of the AA threshold for its role — 4.5:1
// for body/UI text, 3:1 for "meaningful glyphs" (semantic status colors used
// as icon/indicator fills rather than paragraph text). Values are parsed
// straight out of src/tokens.css so this stays correct as the palette moves.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const TOKENS_PATH = fileURLToPath(new URL("../src/tokens.css", import.meta.url));

const TEXT_MIN_RATIO = 4.5;
const GLYPH_MIN_RATIO = 3;

// Backgrounds the app actually paints text/glyphs on.
const BACKGROUNDS = [
  { name: "--c-bg", label: "canvas" },
  { name: "--c-surface-2", label: "surface-2" },
  { name: "--c-surface-3", label: "surface-3" },
];

// Foregrounds used as body/UI text — held to the 4.5:1 text threshold.
const TEXT_FOREGROUNDS = [
  { name: "--c-text", label: "text" },
  { name: "--c-text-muted", label: "muted" },
  { name: "--c-text-faint", label: "faint" },
  { name: "--c-accent-text", label: "accent-text" },
];

// Foregrounds used as meaningful (non-decorative) glyphs/icons only — held
// to the lower 3:1 large-text/graphical-object threshold.
const GLYPH_FOREGROUNDS = [
  { name: "--c-ok", label: "ok" },
  { name: "--c-warn", label: "warn" },
  { name: "--c-danger", label: "danger" },
];

function parseTokens(css) {
  const tokens = new Map();
  const re = /(--[a-z0-9-]+)\s*:\s*(#[0-9a-fA-F]{3,8})\s*;/g;
  for (const match of css.matchAll(re)) {
    tokens.set(match[1], match[2]);
  }
  return tokens;
}

function hexToRgb(hex) {
  let h = hex.slice(1);
  if (h.length === 3 || h.length === 4) {
    h = h
      .split("")
      .map((c) => c + c)
      .join("");
  }
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return { r, g, b };
}

function relativeLuminance({ r, g, b }) {
  const [rs, gs, bs] = [r, g, b].map((c) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  });
  return 0.2126 * rs + 0.7152 * gs + 0.0722 * bs;
}

function contrastRatio(hexA, hexB) {
  const lumA = relativeLuminance(hexToRgb(hexA));
  const lumB = relativeLuminance(hexToRgb(hexB));
  const lighter = Math.max(lumA, lumB);
  const darker = Math.min(lumA, lumB);
  return (lighter + 0.05) / (darker + 0.05);
}

function resolve(tokens, name) {
  const value = tokens.get(name);
  if (!value) {
    throw new Error(`check-contrast: token ${name} not found (or not a solid hex color) in tokens.css`);
  }
  return value;
}

function main() {
  const css = readFileSync(TOKENS_PATH, "utf8");
  const tokens = parseTokens(css);

  const results = [];
  let failures = 0;

  for (const bg of BACKGROUNDS) {
    const bgHex = resolve(tokens, bg.name);

    for (const fg of TEXT_FOREGROUNDS) {
      const fgHex = resolve(tokens, fg.name);
      const ratio = contrastRatio(fgHex, bgHex);
      const pass = ratio >= TEXT_MIN_RATIO;
      if (!pass) failures++;
      results.push({ fg: fg.label, bg: bg.label, ratio, min: TEXT_MIN_RATIO, pass, kind: "text" });
    }

    for (const fg of GLYPH_FOREGROUNDS) {
      const fgHex = resolve(tokens, fg.name);
      const ratio = contrastRatio(fgHex, bgHex);
      const pass = ratio >= GLYPH_MIN_RATIO;
      if (!pass) failures++;
      results.push({ fg: fg.label, bg: bg.label, ratio, min: GLYPH_MIN_RATIO, pass, kind: "glyph" });
    }
  }

  console.log(`check-contrast: ${results.length} pair(s) checked\n`);
  for (const r of results) {
    const status = r.pass ? "PASS" : "FAIL";
    const pair = `${r.fg} on ${r.bg}`.padEnd(28);
    console.log(
      `  [${status}] ${pair} ${r.ratio.toFixed(2)}:1  (min ${r.min}:1, ${r.kind})`,
    );
  }

  if (failures > 0) {
    console.error(`\ncheck-contrast: ${failures} pair(s) below their WCAG 2.2 AA threshold`);
    process.exit(1);
  }

  console.log(`\ncheck-contrast: all pairs pass`);
}

main();
