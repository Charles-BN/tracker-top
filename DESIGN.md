---
name: tracker-top
description: Top-of-pile optimisation for single-axis solar trackers; a dense, exact engineering ledger.
colors:
  survey-blue: "#2a78d6"
  survey-blue-wash: "rgba(42, 120, 214, 0.10)"
  survey-blue-band: "rgba(42, 120, 214, 0.12)"
  sun: "#f5b324"
  ledger-paper: "#f3f2ee"
  sheet-white: "#ffffff"
  ledger-tint: "#f2f1ed"
  instrument-black: "#16191f"
  instrument-ink: "#e9eaee"
  instrument-muted: "#9aa0ab"
  ink: "#0b0b0b"
  ink-2: "#52514e"
  muted: "#898781"
  grid-rule: "#e1e0d9"
  axis-rule: "#c3c2b7"
  hairline: "rgba(11, 11, 11, 0.12)"
  status-compliant: "#0ca30c"
  status-graded: "#fab219"
  status-skipped: "#ec835a"
  status-noncompliant: "#d03b3b"
  status-neutral: "#898781"
  status-notrun: "#c3c2b7"
  text-success: "#006300"
  text-warning: "#8a5a00"
  text-critical: "#b42f2f"
  series-2: "#eb6834"
  series-3: "#1baf7a"
typography:
  display:
    fontFamily: "Segoe UI Variable Text, Segoe UI, system-ui, -apple-system, sans-serif"
    fontSize: "52px"
    fontWeight: 700
    lineHeight: 1.05
    letterSpacing: "-0.03em"
    fontFeature: "tnum"
  headline:
    fontFamily: "Segoe UI Variable Text, Segoe UI, system-ui, -apple-system, sans-serif"
    fontSize: "26px"
    fontWeight: 650
    letterSpacing: "-0.01em"
    fontFeature: "tnum"
  title:
    fontFamily: "Segoe UI Variable Text, Segoe UI, system-ui, -apple-system, sans-serif"
    fontSize: "18px"
    fontWeight: 600
  body:
    fontFamily: "Segoe UI Variable Text, Segoe UI, system-ui, -apple-system, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.5
  table:
    fontFamily: "Segoe UI Variable Text, Segoe UI, system-ui, -apple-system, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    fontFeature: "tnum"
  label:
    fontFamily: "Segoe UI Variable Text, Segoe UI, system-ui, -apple-system, sans-serif"
    fontSize: "11.5px"
    fontWeight: 700
    letterSpacing: "0.06em"
  mono:
    fontFamily: "ui-monospace, Cascadia Mono, Consolas, monospace"
    fontSize: "12.5px"
rounded:
  xs: "4px"
  sm: "6px"
  md: "8px"
  lg: "10px"
  xl: "12px"
  full: "999px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "20px"
  2xl: "24px"
components:
  button:
    backgroundColor: "{colors.sheet-white}"
    textColor: "{colors.ink}"
    rounded: "{rounded.md}"
    padding: "7px 13px"
  button-hover:
    backgroundColor: "{colors.ledger-tint}"
  button-primary:
    backgroundColor: "{colors.survey-blue}"
    textColor: "{colors.sheet-white}"
    rounded: "{rounded.md}"
    padding: "7px 13px"
  button-small:
    rounded: "{rounded.sm}"
    padding: "3px 9px"
  button-ghost:
    textColor: "{colors.ink-2}"
    rounded: "{rounded.md}"
    padding: "7px 13px"
  tab:
    textColor: "{colors.instrument-muted}"
    rounded: "7px"
    padding: "6px 13px"
  tab-active:
    backgroundColor: "{colors.instrument-ink}"
    textColor: "{colors.instrument-black}"
  card:
    backgroundColor: "{colors.sheet-white}"
    rounded: "{rounded.xl}"
    padding: "18px 20px"
  tile:
    backgroundColor: "{colors.sheet-white}"
    rounded: "{rounded.xl}"
    padding: "14px 16px"
  input:
    backgroundColor: "{colors.sheet-white}"
    textColor: "{colors.ink}"
    rounded: "{rounded.sm}"
    padding: "6px 9px"
  chip:
    backgroundColor: "{colors.sheet-white}"
    textColor: "{colors.ink-2}"
    rounded: "{rounded.full}"
    padding: "5px 11px"
  chip-selected:
    backgroundColor: "{colors.survey-blue-wash}"
    textColor: "{colors.survey-blue}"
  table-header:
    backgroundColor: "{colors.ledger-tint}"
    textColor: "{colors.ink-2}"
    typography: "{typography.label}"
    padding: "7px 10px"
  pill:
    rounded: "{rounded.full}"
    padding: "2px 9px"
---

# Design System: tracker-top

## Overview

**Creative North Star: "The Site Office Ledger"**

tracker-top is the record a site engineer opens every morning and trusts. It is a ledger of tens of thousands of piles, kept on warm paper-grey sheets under a dark instrument bar. The figures carry the page: tabular numerals, centimetre precision and honest units. Everything else (chrome, borders, labels) sits back in warm greys so a number, a profile line or a status dot is the first thing the eye lands on.

The system is **crisp, technical and dense**. It is a working tool, used all day on a Windows laptop, and future work leans towards *more* on screen, not less: tighter rows, compact controls, and the feel of a desktop CAD or GIS application rather than a web dashboard. Colour is rationed. Status hues belong to tracker state alone, one blue does all the interactive work, and data views get their own disciplined palettes (categorical, sequential, terrain, diverging) that never borrow the status colours.

What it is not: **a generic SaaS dashboard.** No gradient hero cards, no decorative charts, no marketing flourish, no illustrative empty space. Every element on screen is either a figure, a control, or the label that makes a figure readable.

**Key Characteristics:**
- Warm paper-grey page, white sheets, hairline borders; a near-black instrument bar on top.
- Tabular figures everywhere a number appears; calculated values to 2 decimals, measured ground to 3.
- One interactive blue; status colours reserved for tracker state; data palettes kept separate.
- Uppercase micro-labels (11–12px, tracked out) as the ledger's column headings.
- Compact, bordered, lightly rounded controls; depth comes from borders and tone, not shadow.
- Full light and dark themes, following the OS unless `data-theme` pins one.

## Colors

A warm-neutral ledger with one working blue, a small sun-yellow brand mark, and status colours held strictly in reserve.

### Primary
- **Survey Blue** (`survey-blue`): the only interactive colour. Used for the primary action (*Optimise*), links, focus rings, selected chips, the selected table row (as `survey-blue-wash`), sort arrows, drop-zone hover, and data series 1 (actual progress, profile top of pile). In dark mode it lifts to `#3987e5`.
- **Survey Blue Band** (`survey-blue-band`): translucent fills for allowed bands and ranges in charts (the reveal window in the profile, projection bands in Plots).

### Secondary
- **Sun** (`sun`): the brand's sun in the logo mark and the stale-count badge on the Optimise button. A signature, not a palette colour: never a fill for panels, charts or text.

### Tertiary
- **Series Orange** (`series-2`) and **Series Teal** (`series-3`): data series 2 and 3 in profiles and plots (targets, LIDAR, secondary lines). The full 7-slot categorical set and the sequential, terrain and diverging ramps live in `view3d.js` / `map2d.js` and the sidecar.

### Neutral
- **Ledger Paper** (`ledger-paper`): the page behind everything. Warm, faintly yellowed grey, never pure white. Dark: `#0d0d0d`.
- **Sheet White** (`sheet-white`): cards, tables, inputs, menus, dialogs. Dark: `#1a1a19`.
- **Ledger Tint** (`ledger-tint`): table header rows, hover rows, inner groups (key-value groups, notices, rules), mini-bar tracks. Dark: `#232321`.
- **Instrument Black** (`instrument-black`) with **Instrument Ink** (`instrument-ink`) and **Instrument Muted** (`instrument-muted`): the top bar, its tabs and its text. The active tab inverts to an ink pill on the black bar.
- **Ink** (`ink`), **Ink 2** (`ink-2`), **Muted** (`muted`): three text levels: figures and titles; labels and secondary copy; hints, units and captions. Dark: `#f0efec` / `#c3c2b7` / `#898781`.
- **Grid Rule** (`grid-rule`), **Axis Rule** (`axis-rule`), **Hairline** (`hairline`): table row rules and chart gridlines; chart axes; borders of every card, input and button.

### Status
- **Compliant** (`status-compliant`), **Graded** (`status-graded`), **Skipped / Invalid** (`status-skipped`), **Non-compliant** (`status-noncompliant`), **Awaiting elevations** (`status-neutral`), **Not optimised** (`status-notrun`, drawn as an empty ring). Fills for dots, stacked bars and 3D/map colouring.
- **Text Success / Warning / Critical** (`text-success`, `text-warning`, `text-critical`): the darker text forms of the status hues, for figures and words that must pass contrast (out-of-date flags, ok/bad numbers, pill text, danger buttons).

### Named Rules
**The Reserved Status Rule.** Green, amber, orange and red mean tracker state and nothing else. Data views that colour by a measure (angle, reveal, ground, off-rows distance) use the blue sequential, terrain or blue–red diverging ramps, never the status hues.

**The One Blue Rule.** Survey Blue is the only colour that says "you can act on this" or "this is selected". A second interactive hue is not allowed.

**The Warm Grey Rule.** Neutrals are warm (yellow-tinted) greys. No cool blue-greys in the page body; the cool greys belong to the instrument bar only.

## Typography

**Body Font:** Segoe UI Variable Text (with Segoe UI, system-ui, -apple-system, sans-serif)
**Label/Mono Font:** ui-monospace (with Cascadia Mono, Consolas, monospace) for IDs, file names and code

**Character:** The Windows system face, chosen because it is already on every site laptop and renders crisply at small sizes. It is a single family that works hard: weight and tabular figures create the hierarchy, not a display face.

### Hierarchy
- **Display** (700, 52px, 1.05, −0.03em, tabular): the one hero figure on the Overview (share of trackers with a valid top of pile), with its unit at 26px in Muted.
- **Headline** (650, 26px, −0.01em, tabular): stat-tile values; the related *big figure* is 700 at 34px.
- **Title** (600, 18px): card and section headings (`h2`); `h3` at 15px; import-card titles at 16px.
- **Body** (400, 14px, 1.5): the base size for copy, buttons and inputs. Prose is held to about 60ch.
- **Table** (400, 13px, tabular): every data table; metric values at 20px / 650.
- **Label** (600–700, 11–12px, uppercase, +0.03 to +0.06em): table headers, eyebrows, group titles, limit captions. The ledger's column headings.
- **Small** (12–12.5px): field hints, legends, sub-values, tooltips.

### Named Rules
**The Tabular Figures Rule.** Every number that can sit beside another number uses `font-variant-numeric: tabular-nums` and right alignment in tables. No proportional digits in data.

**The Honest Precision Rule.** Calculated values show 2 decimals, measured ground 3, and units are always present (m, %, °, mm). Never round a figure for looks.

## Layout

- **Page:** a sticky top bar (about 56px) over a single content column, max 1440px wide, padded 24px (16px under 640px). The Map and 3D tabs break out to full width and full height (`100vh − 56px`) with a 290px side panel on the left.
- **Grids:** auto-fit card grids with 16px gaps: 2-up at a 340px minimum, 3-up at 300px, 4-up (Import) at 250px; stat tiles at 160px minimum with 12px gaps.
- **Results:** a two-column master–detail (tracker table about 1fr, detail about 1.2fr) whose detail panel sticks under the bar; it collapses to one column under 1000px.
- **Settings:** a 230px sticky section nav beside the form body, with a sticky translucent save bar at the bottom; the nav becomes a horizontal strip under 800px.
- **Rhythm:** 4 / 8 / 12 / 16 / 20 / 24px. Inner padding of controls is 6–7px vertical; cards 18×20px.
- **Density direction:** new work leans compact. Prefer table rows over cards, 12px gaps over 16px, and side panels over modal steps. The current values are the roomy end of the range, not the floor.

## Elevation & Depth

The ledger is **flat by structure**: depth comes from tone (paper → sheet → tint) and hairline borders. Light mode adds a faint ambient shadow under cards and tiles; dark mode removes it entirely (`--shadow: none`). Real lift is kept for things that float over content.

### Shadow Vocabulary
- **Sheet** (`0 1px 2px rgba(20,22,28,0.05), 0 6px 20px rgba(20,22,28,0.05)`): cards, tiles, the selected settings-nav item, the save bar. Light mode only.
- **Popover** (`0 12px 32px rgba(0,0,0,0.18)`): the Export menu and the Columns chooser.
- **Tooltip** (`0 4px 16px rgba(0,0,0,0.15)`): chart tooltips.
- **Toast** (`0 6px 24px rgba(0,0,0,0.25)`): the inverted ink toast at the bottom centre.

### Named Rules
**The Border-First Rule.** Every surface is bounded by a 1px hairline. Shadow never replaces a border, and nothing at rest uses more than the Sheet shadow.

## Shapes

Gently rounded and consistent, scaled to the element: 4px for bars and small tags, 6px for inputs, small buttons and tooltips, 8px for buttons and notices, 10px for tables, menus, popovers and inner groups, 12px for cards, tiles and dialogs, and full pills for chips, status pills, badges and the sync chip. Status dots are 10px circles; the motor pile is an outlined square marked M. Drop zones use a 2px dashed border. There are no other silhouettes: no clipped corners, blobs or decorative geometry.

## Components

Compact and tool-like: bordered, lightly rounded, no flourish. Affordance comes from borders and state change, never from depth or motion.

### Buttons
- **Shape:** gently rounded (8px); small buttons 6px.
- **Default:** white sheet, ink text, hairline border, 7×13px, weight 500. Hover: Ledger Tint.
- **Primary:** Survey Blue fill and border, white text. Hover: `brightness(1.08)`. One per view, at most (*Optimise* in the bar).
- **Ghost:** transparent, Ink 2 text; tint on hover. **Danger:** default button with critical text.
- **On the instrument bar:** transparent with a 20%-white border and a hover wash.
- **Disabled / busy:** 55% opacity, `progress` cursor.
- **Focus:** 2px Survey Blue outline, 1px offset, on every focusable element.

### Chips
- **Filter chips:** full pill, hairline border, Ink 2 text at 13px over a hidden checkbox. **Selected:** Survey Blue wash, blue border, blue 600 text.
- **Status pills:** full pill at 12px / 600 on a 14% status tint with status text (ok, bad, void in Muted on tint).
- **Tags:** 11.5px / 600 Ink 2 on Ledger Tint, for "optional" and counts.

### Cards / Containers
- **Corner Style:** 12px.
- **Background:** Sheet White on Ledger Paper; inner groups use Ledger Tint at 10px.
- **Shadow Strategy:** the Sheet shadow in light mode, none in dark.
- **Border:** 1px hairline, always.
- **Internal Padding:** 18×20px for cards, 14×16px for tiles. Card heads put the title on the left and tools on the right, baseline-aligned.

### Inputs / Fields
- **Style:** white, hairline border, 6px radius, 6×9px, inheriting the 14px body font.
- **Field layout:** label above in Ink 2 at 12.5px, hint below in Muted at 12px, with 4px gaps.
- **Focus:** a 2px Survey Blue outline.
- **Drop zones:** a 2px dashed hairline box with a bold ink line and a muted hint; on hover or drag-over the border turns blue on the blue wash.

### Navigation
- **Top bar:** Instrument Black, sticky, with the logo and name on the left, then a segmented tab strip (5% white track, 3px inset, 10px radius) with thin separators between groups (Overview | Results Map 3D Plots | Import Settings), then actions on the right.
- **Tabs:** 500 weight in Instrument Muted with a hover wash; the **active** tab inverts to an Instrument Ink pill with black 600 text.
- **Settings nav:** stacked buttons with a title and a muted subtitle; the current item is a white bordered sheet with the Sheet shadow.

### Data Table (signature)
The ledger itself: a 1px-bordered, 10px-radius scroll box. It has a sticky uppercase header row on Ledger Tint (12px / 600, +0.03em), 13px rows with 7×10px cells and a Grid Rule underline, a tint on hover and a blue wash on the selected row. Voided rows are struck through in Muted. Numbers are right-aligned and tabular. A *Columns* popover chooses which columns are shown.

### Status Indicator (signature)
A 10px dot in the status colour beside its label; *Not optimised* is a hollow ring. An "out of date" flag follows in 12px / 600 warning text. Overview and blocks use the same colours as a 2px-gapped stacked bar (14px, or 22px in the hero) with a legend of swatches and counts.

### Tracker Profile (signature)
An SVG elevation profile along the tracker. It draws the survey ground (solid, a dot per surveyed pile), the LIDAR ground (dashed), the top of pile in series blue, the allowed band as a translucent blue fill, and the flood level dashed. Motor piles are outlined squares marked M and the ends are labelled with compass directions (← S, N →). Ticks are 11px Muted and tabular. A full-screen mode lays it over a plain surface.

## Do's and Don'ts

### Do:
- **Do** use tabular numerals and right alignment for every figure, with its unit, at 2 decimals for calculated values and 3 for measured ground.
- **Do** keep Survey Blue (`#2a78d6`, dark `#3987e5`) as the single interactive and selection colour.
- **Do** reserve the status colours for tracker state, and colour measures with the blue sequential, terrain or blue–red diverging ramps.
- **Do** bound every surface with a 1px hairline and keep resting shadows to the Sheet value (none in dark mode).
- **Do** lean compact for new UI: tables over cards, 12px gaps, 6–7px control padding, side panels over modal flows.
- **Do** define every new colour as a token with both light and dark values.
- **Do** use the uppercase micro-label (11–12px, +0.03–0.06em, Muted or Ink 2) for column and group headings.

### Don't:
- **Don't** make it look like a generic SaaS dashboard: no gradient hero cards, decorative charts, illustrations or marketing copy.
- **Don't** use green, amber, orange or red for anything but tracker status (or their text forms for status words and figures).
- **Don't** introduce a second accent hue, or use the Sun yellow outside the logo and the stale-count badge.
- **Don't** use cool blue-grey neutrals in the page body; the warm greys are the ledger.
- **Don't** add web fonts or CDN assets: the system font stack and vendored libraries keep it working offline on locked-down PCs.
- **Don't** round figures for neatness or drop units.
