# UI refresh review — T9

## Current result — source and automated review

**Complete for the current T9 task.** The required source inspection and all automated checks passed.

The previous attempt treated native acceptance as a completion requirement and stopped at privacy denials. The current task explicitly excludes native checks. The historical report below is retained as prior evidence, not a current blocker or a claim of native verification.

### Automated results in this worktree

`cd frontend && npm run check && npm test && npm run build` exited **0** after dependency installation.

- Check: tokens, contrast, theme bootstrap, drag regions, native controls, density, and icon passed; 308 contrast pairs across 11 themes passed.
- Tests: 244 passed in 21 files.
- Build: TypeScript and Vite passed; 119 modules transformed.
- Font output: `frontend/dist/assets/ReadexPro-Variable-B3mkObBa.woff2`, 74,632 bytes (74.63 kB).
- The source and built font have identical SHA-256: `4c244023542b4be1f216c2c677d6bc7ef3e9aceddb939bbe09f51cd97ffa4a92`.

The first command stopped because `vitest` was missing (exit 127). `npm ci --cache /tmp/loopfleet-t9-1939-cache` restored dependencies, then the entire command passed. Installation reported seven dependency vulnerabilities and blocked esbuild/fsevents install scripts; neither prevented the checks or build. No dependencies, lockfiles, tests, or production code changed.

### Source findings

| Requirement | Finding | Result |
| --- | --- | --- |
| Bundled font asset | `frontend/src/assets/fonts/ReadexPro-Variable.woff2` is a valid WOFF2 file. The relative CSS URL produces the identical local asset in the production build; no remote font request is declared. | Pass |
| License | `frontend/src/assets/fonts/ReadexPro-LICENSE.txt` contains the Readex Pro authors' 2019 copyright and complete SIL Open Font License 1.1, including bundling permission and redistribution conditions. This verifies the checked-in license, not a native package license audit. | Pass |
| Font declarations | `frontend/src/tokens.css:114–124` declares normal Readex Pro, variable weights 300–700, WOFF2 format, and `font-display: swap`. `--font-sans` starts with Readex Pro and retains system fallbacks. Weights 400, 500, and 600 fall within the declared range. | Pass |
| Interface inheritance | `frontend/src/main.tsx:5` imports the tokens stylesheet. `tokens.css:1512–1541` applies the sans stack to body and `font: inherit` to buttons, inputs, selects, and textareas. Settings fields/buttons, Select, NumberField, toolbar, and palette input also inherit fonts in their component stylesheets. Headings use weight 600; buttons use 500 where specified. | Pass |
| Monospace roles | `tokens.css` preserves the separate SF Mono/ui-monospace/JetBrains Mono/Menlo stack and applies it to `code` and `pre`. `runview.css` keeps file paths, Diff content, and run IDs monospace; `prd.css` keeps Markdown code and technical metadata monospace. `panels.css` retains technical textareas and agent versions; `palette.css` retains shortcut hints. `grid.css` uses monospace data with an explicit sans role for labels. | Pass |
| Interaction selectors | `panels.css:169–196` guards hover/pressed states with `:not(:disabled)`, uses neutral/primary hover tokens, an accent focus outline, and pressed shadow without translation. Select and NumberField have guarded hover/active, focus, and disabled rules. `shell.css` keeps selected project tint after hover and reveals contextual actions on hover/focus-within. Toolbar, launch controls, grid, dock, and run tabs have focus/hover rules; active run tabs and palette rows preserve selected tint during hover. Disabled palette rows explicitly suppress hover fill. | Pass |
| Reduced motion | `tokens.css:1586` covers all elements and pseudo-elements: automatic scrolling, 0.01ms transitions/animations, and one animation iteration, all with `!important`. `prd.css:141` additionally disables spinner animation. Popover motion falls under the global rule. | Pass |

### Verification boundary

These findings establish CSS declarations, selector behavior in source, license presence, asset bundling, and automated regression results. They do not establish the font WebKit paints or native visual/interaction behavior.

Native rendering, runtime font loading, screenshots, matched-scale comparison, offline startup, HMR, and native interactions remain **unverified**. They are not completion requirements for this task. No native apps, permission probes, screenshots, runtime inspection, or background processes were started in this attempt. Pre-existing processes and the hosting app were untouched.

## Historical report from the previous attempt

### Previous result

**Incomplete. Native release acceptance remains blocked.**

The review stopped at the pre-launch prerequisites on 2026-10-11. No native app or development server launched during this attempt.

The existing refresh changes are present through commit `220b259`. The initial worktree had no tracked changes.

## Pre-launch prerequisites

The rendered profile is:

`~/Library/Application Support/com.loopfleet.app/profiles/f41bd015-2d15-4331-9102-c5c34f9f280c.sb`

| Prerequisite | Observed result |
| --- | --- |
| Native automation grants | Present: `(allow lsopen)`, `(allow appleevent-send)`, and unrestricted `(allow signal)`. The project permission is not disabled. |
| Running host build supports signal grants | The host generated a profile with the unrestricted signal grant. This attempt did not signal the host. |
| Accessibility | `AXIsProcessTrusted()` returned `False`. |
| Screen Recording | `CGPreflightScreenCaptureAccess()` returned `False`. |
| Automation | The System Events probe failed with privilege violation `-10004`. |
| Worktree app and separate app data | Not launched because privacy prerequisites failed. No installed Loopfleet app substituted for this worktree. |
| PID-specific process cleanup | No background processes started. All foreground dependency and validation commands exited. Pre-existing processes remained untouched. |

Exact Automation probe output:

```text
sandbox_extension_issue_file failed for /System/Library/CoreServices/System Events.app: 1 (Operation not permitted)
40:59: execution error: System Events got an error: A privilege violation occurred. (-10004)
```

Exact capture probe output:

```text
sandbox_extension_issue_file failed for /usr/sbin: 1 (Operation not permitted)
could not create image from display
```

The profile grants do not establish macOS privacy approval. These results do not isolate every cause of the Automation denial.
No permissions changed, no sandbox bypass occurred, and no unchanged permission probe repeated.

## Automated results

The final command `cd frontend && npm run check && npm test && npm run build` exited **0**.

| Check | Result |
| --- | --- |
| Design checks | Passed: tokens, contrast, theme bootstrap, drag regions, native controls, density, and icon |
| Theme contrast | All 308 pairs passed across 11 themes |
| Existing tests | All 244 tests passed in 21 files |
| Production build | TypeScript and Vite passed, with 119 modules transformed |
| Bundled font | Build emitted one `ReadexPro-Variable-B3mkObBa.woff2` asset, 74.63 kB |
| Font source comparison | Loopfleet and Esploro WOFF2 SHA-256 hashes match |
| Patch whitespace | `git diff --check` passed |

Matching font hash:

```text
4c244023542b4be1f216c2c677d6bc7ef3e9aceddb939bbe09f51cd97ffa4a92
```

The initial check passed, but the initial test command failed because `vitest` was absent.
The default-cache dependency installation failed with `EPERM` under `~/.npm`.
`npm ci --cache /tmp/loopfleet-t9-f41-cache` succeeded, and the full required command then passed.
The install reported seven dependency vulnerabilities. This task did not change dependencies or the lockfile.

These checks do not prove which font native WebKit paints.

## Native case coverage

Every case below remains **unverified**, not passed. T2 and T4–T8 supply these cases.

The window settings specify 1200×800, but no `minWidth` or `minHeight`.
Thus 1200×800 is the configured initial size, not an enforced minimum.
The native review must distinguish this ambiguity from an observed minimum size.

| Source | Required cases still unverified |
| --- | --- |
| T2 | WebKit loaded font faces and computed Readex Pro at weights 400, 500, and 600 in labels, fields, buttons, menus, and Settings. Startup, offline startup, and CSS HMR. Monospace code and Diff content. |
| T4 | Settings and launch controls: 6px corners, icon alignment, neutral hover, primary token-based hover, visible keyboard focus, pressed state without movement, disabled controls without active hover. Button activation, Select keyboard selection, NumberField keyboard behavior and limits. |
| T5 | Project and plan rows, toolbar actions, and Plan/PRD tabs: neutral hover, persistent selected tint during hover, contextual actions on focus-within. Keyboard navigation, long names, shortcuts, sidebar collapse/restoration, native dragging, and traffic-light clearance. |
| T6 | Every Settings section: icon hierarchy, field alignment, subtle surfaces, 10px theme preview without nested padded cards. Backend Save, unsaved section resets, immediate theme application, local preview highlighting, and cleanup confirmation/cancellation. Overlap and page overflow at both sizes. |
| T7 | Plan, Fleet, run dock, Live Run, and Diff with long labels and active runs. Row density, semantic status colors, destructive-action distinction, row/control hover, persistent tab selection, keyboard focus, execution controls, and acceptance controls. |
| T8 | Command palette, breadcrumb menu, hints, and toast actions in Dark and GitHub Light with reduced motion. 8px popover corners, borders, icon alignment, neutral hover, selected tint, suppressed transitions, keyboard selection, Escape dismissal, restored focus, and accessible hints. |

### Surface and size matrix

Both columns require full workflow comparison in Dark and GitHub Light. No native screenshots exist for this attempt.

| Surface | Configured 1200×800 | 1440×900 |
| --- | --- | --- |
| Settings | Unverified | Unverified |
| Plan | Unverified | Unverified |
| Fleet | Unverified | Unverified |
| Live Run | Unverified | Unverified |
| Diff | Unverified | Unverified |
| Command palette | Unverified | Unverified |

### Theme preview sweep

The existing picker must supply every preview. Static contrast checks passed, but each native preview remains unverified:

| Theme | Native preview |
| --- | --- |
| Dark | Unverified |
| Rosé Pine Moon | Unverified |
| GitHub Dark | Unverified |
| GitHub Light | Unverified |
| Tokyo Night | Unverified |
| Tokyo Night Storm | Unverified |
| Tokyo Night Light | Unverified |
| Tairiki Dark | Unverified |
| Tairiki Light | Unverified |
| Dracula | Unverified |
| Polar Aurora | Unverified |

## Reference comparison and visual limitations

The supplied `~/Desktop/esploro.png` exists, and the Esploro native app exists at `/Applications/Esploro.app`.
Neither establishes a matched-scale comparison with this worktree because native review did not launch.
The same-display comparison at matched logical window size and display scale remains unfinished.

No review screenshot appears under `docs/ui-refresh/` because capture failed before launch.
Existing repository screenshots and earlier font measurements are not evidence for this attempt.

Loaded native Readex, offline startup, HMR, focus, disabled controls, long labels, reduced motion, and all theme previews remain unverified.
No pixel-identical rendering claim applies across environments, even with matching font assets.

T9 remains unchecked in the source plan. Automated success does not satisfy this native release gate.
