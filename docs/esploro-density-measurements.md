# Esploro reference — matched-scale density measurements

Source: `prds/design-refs/esploro_tairiki_dark.png`, measured with a plain
image library (Pillow) reading the PNG header and pixel data directly — no
browser, no OS screen capture, nothing rendered.

These figures back the Phase 2 token retune and the Phase 3 numeric re-check
in `PRD.md` (Stories 18, 27). Every "the reference is tighter by N px" claim
comes from this file, never from eyeballing the two PNGs against each other.

## Scale (device pixel ratio)

- `esploro_tairiki_dark.png` is **3100 × 2002 device px** (PNG IHDR).
- pHYs chunk: 5669 px/m = **144 DPI**, the macOS screenshot convention for a
  **2× Retina** capture (72 DPI = 1×).
- Anchor: the macOS traffic lights are the standard 12 CSS px diameter.
  Measured cluster cores span 26–28 device px (24 px + ~1–2 px anti-alias ring
  each side) at x 130–157 / 176–203 / 222–249, y 94–121 — i.e. **2×**.
- Plausibility: the window is 2876 × 1778 device px = **1438 × 889 CSS px**,
  a normal full-window size on a Retina MacBook; at 1× it would be wider than
  any display.
- `docs/screenshot.png` (the stale hero image) carries the same 144 DPI pHYs
  and the same traffic-light anchors, so both captures are physically 2× —
  the two PNGs differ in *content* (the hero shows an older build with a
  44px titlebar), which is why the app side of this comparison is read from
  `tokens.css`, not from the screenshot.

**Rule: CSS px = device px ÷ 2.**

## Window geometry (device px → CSS px)

- Capture canvas: 3100 × 2002 → 1550 × 1001 CSS.
- App window: x 112..2987, y 76..1853 → 1438 × 889 CSS.
- Sidebar: x 116..751 (636 dev = **318 CSS**, incl. the 1 CSS px divider at
  x 750-751). Left window border 112..115 (2 CSS).
- Main area: x 752..2987 (2236 dev = **1118 CSS**).

## Vertical strips (each separated by a 2-device-px = 1 CSS px hairline, #38383a)

| Strip | device px (top..bottom) | CSS px |
|---|---|---|
| Titlebar (traffic lights in top 28px) | 76..151 = 76 | **38** |
| Tab strip | 152..211 = 60 | **30** |
| Toolbar (buttons 48 dev = 24 CSS tall, ~5.5 CSS vertical margins) | 212..283 = 72 | **36** |
| Sub-tabs | 284..353 = 70 | **35** |
| Filter bar (full-bleed input, bg #202329) | 354..409 = 56 | **28** |
| Column header | 410..485 = 76 | **38** |
| Body (19 full rows + 1 clipped partial) | 486..1751 = 1266 | **633** |
| Pagination | 1752..1815 = 64 | **32** |
| Status bar | 1816..1853 = 38 | **19** |

Strip hairlines verified at x = 752, 900, 1200, 1500, 2000, 2500, 2800, 2980:
150-151, 210-211, 282-283, 352-353, 408-409, 482-485, 1750-1751, 1814-1815.

## Body rows

- Row content 64 dev = **32 CSS**, row separator 2 dev = **1 CSS** hairline
  (#2a2a2c/#2b2b2d), pitch 66 dev = **33 CSS**.
- 19 full rows (486..1739) at exactly 66 dev pitch, then a 10-dev partial row.
- Zebra fills #1c1c1e / #1f1f21 (row 3 at x=1400 reads #212123 — selected or
  hovered row).
- Row text: glyphs 18 dev = 9 CSS tall, vertically centered in the 32 CSS row.
- Selector column: x 752..829 (39 CSS) + vertical hairline 830-831; a
  ~16-dev-wide glyph (checkbox/status) sits centered, x 808..824 — 28 CSS from
  the main-area edge.

## Columns (vertical hairlines in the body)

Separators at x 1190-1191, 1550-1551, 1910-1911, 2270-2271, 2630-2631:
selector col 752..829 (39 CSS), then six ~358-dev (179 CSS) data columns and a
355-dev last column. Data-column text starts ~16-19 dev (8-9.5 CSS) after each
column's left hairline.

## Horizontal insets (CSS px, from the relevant edge)

| Surface | Left | Right |
|---|---|---|
| Body rows / row fills | **0** (full-bleed 752..2986) | **0** (full-bleed) |
| Selector glyph (first row content) | 28 from main edge | — |
| First data-column text | 9 from column start (18 dev) | — |
| Column-header titles | ~12.5 from left divider | 7 (rightmost title ends 2973) |
| Toolbar | 13 (26 dev) | 21 (42 dev) |
| Sub-tabs | 12 (24 dev) | — |
| Filter bar | 0 (full-bleed) | 0 |
| Pagination | 20 (40 dev) | 26 (52 dev) |
| Status bar | 12 (24 dev) | 12.5 (25 dev) |
| Titlebar right cluster | — | 8 (16 dev) |
| Sidebar text | 8 (16 dev from window edge) | — (runs to the divider) |

## loopfleet's current values (tokens.css `:root`, read 2026-09-02)

| Token | loopfleet | Reference | Diff |
|---|---|---|---|
| `--sidebar-w` | 268px | 318 | loopfleet **50px narrower** |
| `--titlebar-h` | 38px | 38 | **match** |
| `--toolbar-h` | 44px | 36 | loopfleet **8px taller** |
| `--control-h` | 28px | filter bar 28 = match; tab strip 30 | at/under reference |
| `--control-h-lg` | 30px | toolbar buttons 24 | loopfleet **6px taller** |
| `--row-h` | 30px | 33 pitch (32 content + 1 hairline) | loopfleet **2-3px tighter** |

Notes for the retune task:
- The reference's own row strip is a database grid; loopfleet's `--row-h`
  drives task rows, which the PRD's problem statement faults for *wrapping*
  (variable height), not for the base height — the token is already at or
  under the reference's pitch.
- The reference's compact strip rhythm is 28-38 CSS; the toolbar (36) is the
  only strip loopfleet's `--toolbar-h` (44) clearly overshoots.
- Panel-padding band: reference row insets run ~8-13 CSS (loopfleet
  `--space-2/3` = 8/12); the reference has no card padding analogue above
  that (settings cards are the Phase 3 target).
