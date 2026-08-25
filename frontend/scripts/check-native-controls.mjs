#!/usr/bin/env node
// Native control check: a raw `<select>` or `<input type="number">` renders
// with unthemeable browser chrome that doesn't match the rest of the app —
// that's why Select.tsx and NumberField.tsx exist. Both primitives (and their
// own native `<input>`/`<button>` internals) are exempt; every other source
// file is scanned so a reintroduced native control fails `npm run check`
// instead of quietly shipping mismatched chrome.

import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join, relative } from "node:path";

const SRC_DIR = fileURLToPath(new URL("../src/", import.meta.url));

// The primitives are allowed to contain the native elements they wrap.
const EXEMPT_FILES = new Set(["components/Select.tsx", "components/NumberField.tsx"]);

const SELECT_RE = /<select[\s>]/;
const NUMBER_INPUT_RE = /<input\b[^>]*\btype\s*=\s*["']number["']/;

const errors = [];

function listSourceFiles(dir) {
  const files = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) files.push(...listSourceFiles(path));
    else if (entry.isFile() && (entry.name.endsWith(".tsx") || entry.name.endsWith(".ts")))
      files.push(path);
  }
  return files;
}

function checkFile(path) {
  const relPath = relative(SRC_DIR, path);
  if (EXEMPT_FILES.has(relPath)) return;

  const contents = readFileSync(path, "utf8");
  contents.split("\n").forEach((line, i) => {
    if (SELECT_RE.test(line)) {
      errors.push(
        `src/${relPath}:${i + 1}: raw <select> — use <Select> from` +
          ` components/Select.tsx instead:\n      ${line.trim()}`,
      );
    }
    if (NUMBER_INPUT_RE.test(line)) {
      errors.push(
        `src/${relPath}:${i + 1}: raw <input type="number"> — use` +
          ` <NumberField> from components/NumberField.tsx instead:\n      ${line.trim()}`,
      );
    }
  });
}

const files = listSourceFiles(SRC_DIR);
for (const path of files) checkFile(path);

if (errors.length) {
  console.error("Native control check failed:\n");
  for (const error of errors) console.error(`  ${error}`);
  console.error(
    "\nNative <select> and <input type=\"number\"> render with unthemeable" +
      "\nbrowser chrome. Use the Select and NumberField primitives instead.",
  );
  process.exit(1);
}

console.log(`Native control check passed (${files.length} files clean).`);
