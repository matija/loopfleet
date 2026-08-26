#!/usr/bin/env node
// Contrast check: computes WCAG 2.2 relative-luminance contrast ratios for
// every foreground/background pair the app actually uses, and fails the
// build if any pair falls short of the AA threshold for its role — 4.5:1
// for body/UI text, 3:1 for "meaningful glyphs" (semantic status colors used
// as icon/indicator fills rather than paragraph text). Values are parsed
// straight out of src/tokens.css so this stays correct as the palette moves.
//
// Every theme is checked, not just the default: tokens.css carries one color
// block per theme (`:root, [data-theme="dark"]`, `[data-theme="rose-pine-moon"]`,
// …) and each block re-tints the same token names, so a flat parse of the file
// would only ever measure whichever block is written last. The registry in
// src/themes.ts is the list this walks, so adding a theme there without giving
// it an accessible palette fails the check rather than shipping unmeasured.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const TOKENS_PATH = fileURLToPath(new URL("../src/tokens.css", import.meta.url));
const THEMES_PATH = fileURLToPath(new URL("../src/themes.ts", import.meta.url));

const TEXT_MIN_RATIO = 4.5;
const GLYPH_MIN_RATIO = 3;

// Backgrounds the app actually paints text/glyphs on. --c-surface-hover is a
// translucent fill rather than a solid step, so it's listed with the surface it
// covers (rows and controls hover over the base field) and flattened against it
// before measuring — a hovered row is a real reading surface, not a decoration.
const BACKGROUNDS = [
  { name: "--c-bg", label: "canvas" },
  { name: "--c-surface-2", label: "surface-2" },
  { name: "--c-surface-3", label: "surface-3" },
  { name: "--c-surface-hover", over: "--c-bg", label: "hover on canvas" },
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

// The themes a picker can offer, in registry order, read from the same
// THEMES literal that check-theme-bootstrap validates against index.html.
function parseThemes(ts) {
  const block = ts.match(/export const THEMES[^=]*=\s*\[([\s\S]*?)\]/);
  if (!block) {
    throw new Error("check-contrast: could not parse THEMES from src/themes.ts");
  }
  const themes = [...block[1].matchAll(/id:\s*"([^"]+)",\s*label:\s*"([^"]+)"/g)].map(
    (m) => ({ id: m[1], label: m[2] }),
  );
  if (themes.length === 0) {
    throw new Error("check-contrast: src/themes.ts declares no themes");
  }
  return themes;
}

function parseTokens(css) {
  const tokens = new Map();
  const re = /(--[a-z0-9-]+)\s*:\s*(#[0-9a-fA-F]{3,8}|rgba?\([^)]*\))\s*;/g;
  for (const match of css.matchAll(re)) {
    tokens.set(match[1], match[2]);
  }
  return tokens;
}

// Solid-hex tokens defined by the rule blocks that apply to `themeId`. A
// theme's block is the one whose selector list names it via [data-theme="id"];
// declarations are collected in file order so a later block still wins for the
// same theme, exactly as the cascade would resolve it.
function tokensForTheme(css, themeId) {
  const tokens = new Map();
  let found = false;
  for (const rule of css.matchAll(/([^{}]*)\{([^{}]*)\}/g)) {
    const [, selector, body] = rule;
    if (!selector.includes(`[data-theme="${themeId}"]`)) continue;
    found = true;
    for (const [name, value] of parseTokens(body)) {
      tokens.set(name, value);
    }
  }
  if (!found) {
    throw new Error(
      `check-contrast: src/tokens.css has no [data-theme="${themeId}"] block` +
        ` — every theme in src/themes.ts needs its own color block`,
    );
  }
  return tokens;
}

// Parses either a solid hex or an rgb()/rgba() literal into {r,g,b,a}.
function parseColor(value) {
  if (value.startsWith("#")) return { ...hexToRgb(value), a: 1 };
  const parts = value
    .slice(value.indexOf("(") + 1, value.lastIndexOf(")"))
    .split(/[,/\s]+/)
    .filter(Boolean)
    .map(Number);
  const [r, g, b, a = 1] = parts;
  if ([r, g, b, a].some((n) => !Number.isFinite(n))) {
    throw new Error(`check-contrast: could not parse color ${value}`);
  }
  return { r, g, b, a };
}

// Flattens a translucent fill onto an opaque backdrop (simple source-over).
function composite(fg, bg) {
  const mix = (f, b) => Math.round(f * fg.a + b * (1 - fg.a));
  return { r: mix(fg.r, bg.r), g: mix(fg.g, bg.g), b: mix(fg.b, bg.b), a: 1 };
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

function contrastRatio(colorA, colorB) {
  const lumA = relativeLuminance(colorA);
  const lumB = relativeLuminance(colorB);
  const lighter = Math.max(lumA, lumB);
  const darker = Math.min(lumA, lumB);
  return (lighter + 0.05) / (darker + 0.05);
}

// The opaque color a background entry actually presents to text: itself when
// solid, or itself flattened onto its `over` backdrop when translucent.
function resolveBackground(tokens, bg, themeId) {
  const fill = parseColor(resolve(tokens, bg.name, themeId));
  if (fill.a >= 1) return fill;
  if (!bg.over) {
    throw new Error(
      `check-contrast: ${bg.name} is translucent but has no "over" backdrop declared`,
    );
  }
  return composite(fill, parseColor(resolve(tokens, bg.over, themeId)));
}

function resolve(tokens, name, themeId) {
  const value = tokens.get(name);
  if (!value) {
    throw new Error(
      `check-contrast: token ${name} not found (or not a solid hex color) in` +
        ` the [data-theme="${themeId}"] block of tokens.css`,
    );
  }
  return value;
}

function checkTheme(css, theme) {
  const tokens = tokensForTheme(css, theme.id);
  const results = [];

  for (const bg of BACKGROUNDS) {
    const bgColor = resolveBackground(tokens, bg, theme.id);

    for (const fg of TEXT_FOREGROUNDS) {
      const fgColor = parseColor(resolve(tokens, fg.name, theme.id));
      const ratio = contrastRatio(fgColor, bgColor);
      results.push({
        fg: fg.label,
        bg: bg.label,
        ratio,
        min: TEXT_MIN_RATIO,
        pass: ratio >= TEXT_MIN_RATIO,
        kind: "text",
      });
    }

    for (const fg of GLYPH_FOREGROUNDS) {
      const fgColor = parseColor(resolve(tokens, fg.name, theme.id));
      const ratio = contrastRatio(fgColor, bgColor);
      results.push({
        fg: fg.label,
        bg: bg.label,
        ratio,
        min: GLYPH_MIN_RATIO,
        pass: ratio >= GLYPH_MIN_RATIO,
        kind: "glyph",
      });
    }
  }

  return results;
}

function main() {
  const css = readFileSync(TOKENS_PATH, "utf8");
  const themes = parseThemes(readFileSync(THEMES_PATH, "utf8"));

  let checked = 0;
  let failures = 0;

  for (const theme of themes) {
    const results = checkTheme(css, theme);
    checked += results.length;

    console.log(`check-contrast: ${theme.label} (${theme.id}) — ${results.length} pair(s)\n`);
    for (const r of results) {
      if (!r.pass) failures++;
      const status = r.pass ? "PASS" : "FAIL";
      const pair = `${r.fg} on ${r.bg}`.padEnd(32);
      console.log(
        `  [${status}] ${pair} ${r.ratio.toFixed(2)}:1  (min ${r.min}:1, ${r.kind})`,
      );
    }
    console.log("");
  }

  if (failures > 0) {
    console.error(
      `check-contrast: ${failures} of ${checked} pair(s) below their WCAG 2.2 AA threshold`,
    );
    process.exit(1);
  }

  console.log(
    `check-contrast: all ${checked} pair(s) pass across ${themes.length} theme(s)`,
  );
}

main();
