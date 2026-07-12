---
name: Loopfleet
description: A quiet control desk for supervising coding-agent runs.
colors:
  canvas: "#0d0d10"
  surface: "#131316"
  surface-raised: "#1a1a1f"
  surface-active: "#232329"
  border: "#26262c"
  border-strong: "#37373f"
  text: "#ececee"
  text-muted: "#a0a0a8"
  text-faint: "#7d7d86"
  primary: "#6275f2"
  primary-quiet: "#23294d"
  success: "#3fb950"
  warning: "#d29922"
  danger: "#f85149"
  danger-quiet: "#3a1d1d"
typography:
  title:
    fontFamily: "Readex Pro, -apple-system, BlinkMacSystemFont, SF Pro Text, Segoe UI, system-ui, sans-serif"
    fontSize: "18px"
    fontWeight: 600
    lineHeight: 1.3
  body:
    fontFamily: "Readex Pro, -apple-system, BlinkMacSystemFont, SF Pro Text, Segoe UI, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.5
  label:
    fontFamily: "Readex Pro, -apple-system, BlinkMacSystemFont, SF Pro Text, Segoe UI, system-ui, sans-serif"
    fontSize: "12px"
    fontWeight: 500
    lineHeight: 1.3
rounded:
  sm: "6px"
  md: "8px"
  lg: "12px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "24px"
  xxl: "32px"
components:
  button-primary:
    backgroundColor: "{colors.primary-quiet}"
    textColor: "{colors.text}"
    rounded: "{rounded.md}"
    padding: "4px 12px"
  button-secondary:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.text-muted}"
    rounded: "{rounded.md}"
    padding: "4px 12px"
  input:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    rounded: "{rounded.md}"
    padding: "4px 8px"
  navigation-selected:
    backgroundColor: "{colors.primary-quiet}"
    textColor: "{colors.text}"
    rounded: "{rounded.md}"
    padding: "8px 12px"
---

# Design System: Loopfleet

## Overview

**Creative North Star: "The Quiet Control Desk"**

Loopfleet is a compact desktop work surface for people supervising autonomous coding work. Tonal near-black layers establish hierarchy without decorative effects; precise spacing, stable controls, and familiar interaction patterns keep attention on tasks and agent state.

The system is restrained rather than austere. Information may be dense when it helps comparison or scanning, but every visual element must earn its place. It explicitly rejects cluttered dashboards, decorative SaaS styling, terminal cosplay, gratuitous cards, and interactions that move surrounding content.

**Key Characteristics:**
- Compact, stable, and familiar controls
- Tonal separation with fine borders
- One restrained action accent
- Semantic color only for actionable status
- Dense logs with clear alignment and minimal padding

## Colors

The palette is a neutral near-black field with a reserved indigo action accent and conventional semantic states.

### Primary
- **Control Indigo:** Used only for primary actions, keyboard focus, active selection, and in-progress state.
- **Recessed Indigo:** Used behind selected navigation and quiet primary controls.

### Neutral
- **Cockpit Canvas:** The deepest application background.
- **Instrument Surface:** The default sidebar, panel, and content surface.
- **Raised Surface:** Toolbars, compact rows, and secondary controls.
- **Active Surface:** Selected or emphasized neutral regions.
- **Primary Ink:** Main labels and content.
- **Muted Ink:** Secondary information that must remain readable.
- **Faint Ink:** Tertiary metadata only; never placeholder or body copy when contrast would fail.

### Named Rules

**The One Accent Rule.** Indigo is the only non-semantic accent and must remain under roughly ten percent of a screen.

**The Semantic Color Rule.** Green, amber, and red communicate success, warning, and danger only. They are forbidden as decoration.

## Typography

**Display Font:** Readex Pro with system sans-serif fallbacks  
**Body Font:** Readex Pro with system sans-serif fallbacks  
**Label/Mono Font:** SF Mono with JetBrains Mono and Menlo fallbacks

**Character:** A single readable sans keeps the product coherent; monospace is reserved for paths, commands, timestamps, and structured data.

### Hierarchy
- **Headline** (600, 18px, 1.3): Main view titles.
- **Title** (600, 15px, 1.3): Panel and task headings.
- **Body** (400, 13px, 1.5): Task text and explanatory content, capped near 72 characters where prose is shown.
- **Label** (500–600, 11–12px, 1.3): Controls, metadata, compact status, and table headers.

### Named Rules

**The Syntax Boundary Rule.** Markdown delimiters and implementation syntax never appear in user-facing task text.

## Elevation

The system is flat by default. Depth comes from adjacent tonal layers and one-pixel borders; the existing ambient shadow is reserved for overlays such as command palettes and toasts.

### Shadow Vocabulary
- **Overlay:** A broad, dark ambient shadow for transient UI that floats above the app shell.

### Named Rules

**The Tonal Layer Rule.** Persistent panels never use shadows to compete for attention.

## Components

### Buttons
- **Shape:** Compact, gently curved controls (8px radius) with a stable one-pixel border.
- **Primary:** Recessed indigo background, readable ink, and 4px by 12px padding.
- **Hover / Focus:** Color and border changes only; dimensions, padding, and position never change. Focus uses a visible indigo outline.
- **Secondary / Ghost:** Neutral tonal backgrounds or transparent surfaces with muted ink.

### Chips
- **Style:** Small status labels use quiet tonal fills and semantically correct text.
- **State:** Active and selected chips may use recessed indigo; inactive chips stay neutral.

### Cards / Containers
- **Corner Style:** Soft but compact (8–12px radius).
- **Background:** One neutral layer appropriate to hierarchy.
- **Shadow Strategy:** None for persistent surfaces.
- **Border:** One-pixel neutral divider.
- **Internal Padding:** 12–16px; avoid nested padded cards.

### Inputs / Fields
- **Style:** Native, familiar controls with neutral fill, one-pixel border, and 8px radius.
- **Focus:** Indigo outline or border without changing dimensions.
- **Error / Disabled:** Semantic danger only for actual errors; disabled controls retain readable labels.

### Navigation
- **Style:** Compact rows with muted default text, neutral hover fill, and recessed-indigo selection. Hover never changes layout geometry.

### Event Log

Rows are mono-spaced, aligned, and compact. Type is communicated with restrained labels and text weight; multiple unrelated hues are prohibited. Vertical padding must stay at 2–4px so the stream reads as a log rather than a stack of cards.

## Do's and Don'ts

### Do:
- **Do** preserve the near-black neutral hierarchy and use Control Indigo only for action, focus, and selection.
- **Do** keep all hover and active states spatially stable.
- **Do** use compact rows and familiar native input behavior.
- **Do** normalize authored task text before displaying it.
- **Do** meet WCAG 2.2 AA contrast and support reduced motion.

### Don't:
- **Don't** build cluttered dashboards or use decorative SaaS styling.
- **Don't** imitate a terminal for atmosphere.
- **Don't** create gratuitous cards or nest padded containers.
- **Don't** use saturated colors without semantic meaning.
- **Don't** let hover effects move surrounding content.
- **Don't** use awkward custom form controls where a familiar stepper or select is clearer.
- **Don't** expose raw Markdown such as `**` in task labels, headers, or run context.
