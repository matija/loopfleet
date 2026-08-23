#!/usr/bin/env node
// index.html/themes.ts sync check.
//
// index.html runs a tiny inline script before the bundle loads that reads the
// persisted theme and sets data-theme, so the first paint is already in the
// right palette instead of flashing the default. That script can't import
// src/themes.ts — a module script would defer past first paint — so it carries
// its own copy of the storage key, the id list, and the default id.
//
// This check is what keeps the copy honest: add or rename a theme in
// themes.ts without updating index.html and `npm run check` fails, instead of
// the new theme silently flashing dark on every launch.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const themesPath = fileURLToPath(new URL("../src/themes.ts", import.meta.url));
const htmlPath = fileURLToPath(new URL("../index.html", import.meta.url));

const themes = readFileSync(themesPath, "utf8");
const html = readFileSync(htmlPath, "utf8");

const errors = [];

function fail(message) {
  errors.push(message);
}

// --- what themes.ts declares ---

const idsBlock = themes.match(/export const THEMES[^=]*=\s*\[([\s\S]*?)\]/);
const registryIds = idsBlock
  ? [...idsBlock[1].matchAll(/id:\s*"([^"]+)"/g)].map((m) => m[1])
  : null;
const registryKey = themes.match(/THEME_STORAGE_KEY\s*=\s*"([^"]+)"/)?.[1];
const registryDefault = themes.match(
  /DEFAULT_THEME_ID:\s*ThemeId\s*=\s*"([^"]+)"/,
)?.[1];

if (!registryIds?.length) fail("src/themes.ts: could not parse THEMES ids");
if (!registryKey) fail("src/themes.ts: could not parse THEME_STORAGE_KEY");
if (!registryDefault) fail("src/themes.ts: could not parse DEFAULT_THEME_ID");

// --- what index.html's bootstrap repeats ---

const htmlIdsRaw = html.match(/var THEMES = (\[[^\]]*\]);/)?.[1];
const htmlKey = html.match(/localStorage\.getItem\("([^"]+)"\)/)?.[1];
const htmlDefault = html.match(/=== -1 \? "([^"]+)" : stored/)?.[1];

if (!htmlIdsRaw) fail("index.html: no `var THEMES = [...]` in the bootstrap");
if (!htmlKey) fail("index.html: bootstrap doesn't read a localStorage key");
if (!htmlDefault) fail("index.html: bootstrap has no default-id fallback");

// --- compare ---

if (registryIds?.length && htmlIdsRaw) {
  let htmlIds;
  try {
    htmlIds = JSON.parse(htmlIdsRaw);
  } catch {
    fail(`index.html: THEMES is not valid JSON: ${htmlIdsRaw}`);
  }
  if (htmlIds && JSON.stringify(htmlIds) !== JSON.stringify(registryIds)) {
    fail(
      `theme ids out of sync:\n  src/themes.ts: ${JSON.stringify(registryIds)}` +
        `\n  index.html:    ${JSON.stringify(htmlIds)}`,
    );
  }
}

if (registryKey && htmlKey && registryKey !== htmlKey) {
  fail(
    `storage key out of sync: src/themes.ts has "${registryKey}", ` +
      `index.html reads "${htmlKey}"`,
  );
}

if (registryDefault && htmlDefault && registryDefault !== htmlDefault) {
  fail(
    `default theme out of sync: src/themes.ts has "${registryDefault}", ` +
      `index.html falls back to "${htmlDefault}"`,
  );
}

if (errors.length) {
  console.error("Theme bootstrap check failed:\n");
  for (const error of errors) console.error(`  ${error}`);
  console.error(
    "\nThe inline script in index.html must mirror src/themes.ts exactly.",
  );
  process.exit(1);
}

console.log(
  `Theme bootstrap check passed (${registryIds.length} themes, key "${registryKey}").`,
);
