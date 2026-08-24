#!/usr/bin/env node
// Drag-region check: the window has no native title bar to grab, so the strip
// under the window's top edge — the sidebar's `.sidebar__top` on the left and
// the main pane's `.toolbar` beside it — is the only thing that can move the
// window. That strip works only if three separate pieces stay in place, in
// three files that are easy to change independently:
//
//   1. src-tauri/capabilities/default.json grants the window-drag permissions.
//      Without `core:window:allow-start-dragging` the drag is rejected at the
//      IPC boundary and the window simply won't move.
//   2. No CSS file declares `app-region` / `-webkit-app-region`. Those are
//      Electron/Chromium properties; the app's WKWebView ignores them, so a
//      reintroduced `app-region: drag` looks like it makes something draggable
//      while doing nothing — and `app-region: no-drag` looks like an opt-out
//      that isn't one. The real mechanism is `data-tauri-drag-region`.
//   3. Both halves of the strip carry `data-tauri-drag-region="deep"`, so a
//      press anywhere in the subtree drags (the bare attribute only counts
//      presses landing on the strip element itself).
//
// Each of these has already been lost once. This check turns the next
// regression into a failed `npm run check` instead of a window that can't be
// moved.

import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join, relative } from "node:path";

const SRC_DIR = fileURLToPath(new URL("../src/", import.meta.url));
const CAPABILITIES_PATH = fileURLToPath(
  new URL("../../src-tauri/capabilities/default.json", import.meta.url),
);

// `allow-start-dragging` backs the drag itself; `allow-toggle-maximize` backs
// the double-click-the-titlebar-to-zoom that Tauri's drag handler performs on
// the same elements. Losing either breaks a gesture the strip is expected to
// serve.
const REQUIRED_PERMISSIONS = [
  "core:window:allow-start-dragging",
  "core:window:allow-toggle-maximize",
];

const APP_REGION_RE = /(?:-webkit-)?app-region\s*:/;

// The two elements that make up the strip: source file, the className that
// identifies the element, and the attribute value it must carry.
const DRAG_REGION_ELEMENTS = [
  { file: "src/components/AppShell.tsx", className: "sidebar__top" },
  { file: "src/components/Toolbar.tsx", className: "toolbar" },
];
const REQUIRED_ATTRIBUTE = 'data-tauri-drag-region="deep"';

const errors = [];

function fail(message) {
  errors.push(message);
}

function read(path) {
  try {
    return readFileSync(path, "utf8");
  } catch (err) {
    fail(`${path}: cannot read (${err.code ?? err.message})`);
    return null;
  }
}

// --- 1. capabilities ---

function checkCapabilities() {
  const raw = read(CAPABILITIES_PATH);
  if (raw === null) return;

  let capabilities;
  try {
    capabilities = JSON.parse(raw);
  } catch (err) {
    fail(`src-tauri/capabilities/default.json: invalid JSON (${err.message})`);
    return;
  }

  const permissions = capabilities.permissions;
  if (!Array.isArray(permissions)) {
    fail("src-tauri/capabilities/default.json: no `permissions` array");
    return;
  }

  for (const permission of REQUIRED_PERMISSIONS) {
    if (!permissions.includes(permission)) {
      fail(
        `src-tauri/capabilities/default.json: missing "${permission}" — the` +
          ` titlebar strip can't move the window without it`,
      );
    }
  }
}

// --- 2. no app-region in CSS ---

function listCssFiles(dir) {
  const files = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) files.push(...listCssFiles(path));
    else if (entry.isFile() && entry.name.endsWith(".css")) files.push(path);
  }
  return files;
}

function checkCss() {
  const cssFiles = listCssFiles(SRC_DIR);
  for (const path of cssFiles) {
    const contents = read(path);
    if (contents === null) continue;
    contents.split("\n").forEach((line, i) => {
      if (APP_REGION_RE.test(line)) {
        fail(
          `src/${relative(SRC_DIR, path)}:${i + 1}: \`app-region\` does` +
            ` nothing in this app's WebView —` +
            ` use \`data-tauri-drag-region\` instead:\n      ${line.trim()}`,
        );
      }
    });
  }
  return cssFiles.length;
}

// --- 3. the strip's drag attributes ---

// Returns the source text of the JSX opening tag containing `index`, i.e. the
// span from the `<` that opens it to the `>` that closes it. Attribute values
// never contain a bare `<`/`>` in these files, so scanning outward is enough —
// and it survives the tag being wrapped across lines by the formatter.
function openingTagAt(source, index) {
  const start = source.lastIndexOf("<", index);
  const end = source.indexOf(">", index);
  if (start === -1 || end === -1) return null;
  return source.slice(start, end + 1);
}

function checkDragAttributes() {
  for (const { file, className } of DRAG_REGION_ELEMENTS) {
    const path = fileURLToPath(new URL(`../${file}`, import.meta.url));
    const source = read(path);
    if (source === null) continue;

    // Skip the file's header comment, which mentions these class names in
    // prose; only the JSX below it should be matched.
    const marker = `className="${className}"`;
    const index = source.indexOf(marker);
    if (index === -1) {
      fail(`${file}: no element with ${marker} — is the strip still there?`);
      continue;
    }

    const tag = openingTagAt(source, index);
    if (tag === null || !tag.includes(REQUIRED_ATTRIBUTE)) {
      fail(
        `${file}: the .${className} element must carry` +
          ` ${REQUIRED_ATTRIBUTE} — without it that half of the titlebar` +
          ` strip no longer drags the window`,
      );
    }
  }
}

// --- run ---

checkCapabilities();
const cssFileCount = checkCss();
checkDragAttributes();

if (errors.length) {
  console.error("Drag region check failed:\n");
  for (const error of errors) console.error(`  ${error}`);
  console.error(
    "\nThe window has no native title bar: the strip under the top edge is" +
      "\nthe only way to move it. See the drag-region notes in" +
      " AppShell.tsx.",
  );
  process.exit(1);
}

console.log(
  `Drag region check passed (${REQUIRED_PERMISSIONS.length} permissions,` +
    ` ${cssFileCount} CSS files clean,` +
    ` ${DRAG_REGION_ELEMENTS.length} strip elements draggable).`,
);
