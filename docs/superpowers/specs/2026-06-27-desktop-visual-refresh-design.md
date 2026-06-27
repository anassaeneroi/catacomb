# Desktop Visual Refresh + Theme Pack

**Date:** 2026-06-27
**Status:** Approved (design phase)
**Scope:** Desktop (egui) UI only. Web UI is out of scope.

## Goal

Lift the desktop UI's out-of-box experience and expand its identity options. Two
prongs: (1) add 12 new themes, (2) refresh the whole look — default themes
included — so the app no longer reads as stock egui.

## Current state (findings from code survey)

- `theme.rs` ships **7 themes**: Dark, Light, Dracula, Trans, Emo:Nocturnal,
  Emo:Coffin, Emo:Scene Queen. Dark and Light are **stock
  `egui::Visuals::dark()`/`light()` with zero customization**; the other five
  are fully hand-tuned.
- Video list is **flat horizontal rows** (thumb left, text right) — no card
  background, no rounded corners. Reads as a spreadsheet, not a media library.
- **Accent colors are hardcoded** throughout `app.rs`, ignoring the active theme:
  - selected ring: `Color32::from_rgb(120, 170, 230)` (`app.rs:4225`)
  - playing ring:  `Color32::from_rgb(110, 200, 110)` (`app.rs:4232`)
  - bulk-checked ring: `Color32::from_rgb(180, 130, 240)` (`app.rs:4239`)
  So even in a hand-tuned theme, selection always glows stock blue.
- Placeholder thumbnails are inconsistent: channel cards use `📺` on
  `from_gray(30)` (`app.rs:3972`); video cards use `▶` on `from_gray(38)`
  (`app.rs:4210`).
- Density is very high (~12px text, ~10px padding) with no typographic
  hierarchy — section headers read the same weight/size as body text.

## Design

### 1. Theme pack — 12 new themes

Add twelve fully-tuned `Visuals` to `theme.rs`, each defining panel fills,
per-state widget strokes (noninteractive/inactive/hovered/active/open),
selection bg+stroke, hyperlink color, warn/error colors — the full set the
existing five themed variants already tune. Catalog grows from 7 to **19**.

| # | Category | Theme key | Identity |
|---|----------|-----------|----------|
| 1 | Neon | `cyberpunk` | Magenta + electric cyan on black. Hacker HUD. |
| 2 | Neon | `synthwave` | Sunset gradient (hot pink → orange → purple) on deep indigo. |
| 3 | Neon | `vaporwave` | Pastel pink + cyan + lavender on deep plum. Aesthetic. |
| 4 | Goth | `cemetery-moss` | Weathered stone + mossy green + bone. Organic, ancient. |
| 5 | Goth | `vampire` | Deep wine burgundy + antique gold + black. Regal. |
| 6 | Goth | `witching-hour` | Midnight indigo + moonlight silver + arcane violet. Mystical. |
| 7 | Dev | `nord` | Arctic blues & greys. Cold, legible. |
| 8 | Dev | `gruvbox` | Warm earthy retro groove. Contrast-focused. |
| 9 | Dev | `tokyo-night` | Tokyo city lights. Blue/purple, clean. |
| 10 | Cozy | `paper` | Aged paper + sepia ink. Quiet reading-room. |
| 11 | Cozy | `honey` | Warm amber + gold + cream. Golden-hour. |
| 12 | Cozy | `candlelight` | Dim warm glow + toasted brown. Evening, intimate. |

Deliberate distinctness: Vampire is kept visually separate from the existing
Emo:Coffin (burgundy+gold vs blood-red+black); Witching Hour separate from
Emo:Nocturnal (silver+violet vs hot-pink). Dracula is not re-added (already
shipped).

Each theme **must** export three semantic accent colors (see §2) in addition to
the standard `Visuals` fields.

### 2. Theme-aware semantic accents (bug fix)

Today's hardcoded rings ignore the theme. Replace with named semantic accents
that each theme provides:

- `accent` — selection / focus (replaces the `120,170,230` blue)
- `success` — playing / watched (replaces `110,200,110` green)
- `warning` — bulk-selection highlight (replaces `180,130,240` purple)

**Mechanism:** egui `Visuals` has no field for arbitrary semantic accents, so
expose them via a `ThemeAccents { accent: Color32, success: Color32, warning:
Color32 }` struct in `theme.rs`, with a `pub fn accents_for(name: &str) ->
ThemeAccents` lookup. `App` holds a `theme_accents: ThemeAccents` field,
recomputed at every site that already calls `theme::apply(ctx, name)` (App
construction + the Settings theme-picker callback). All paint code that
currently hardcodes a color reads from `self.theme_accents.accent` /
`.success` / `.warning` instead. This makes the new themes look correct and
also makes the existing Dracula/Trans/Emo themes' selection rings match their
palettes.

### 3. View-mode toggle: List / Card / Grid

Three render paths for the video list, switchable live via a toolbar toggle.

- **List** — current horizontal rows, polished (theme-aware rings, unified
  placeholder, refined spacing). The lowest-disruption default.
- **Card** — same horizontal layout, but each row is a rounded card on the
  faint-bg fill with a hover lift. Media-app feel without sacrificing density.
- **Grid** — YouTube/Plex-style vertical cards (thumb on top, title + meta
  below), responsive column count derived from window width. Most visual.

**Persistence model (global default + per-view override):**

- **Global default** stored in `config.toml` (new
  `[ui] default_view_mode = "list|card|grid"`) and seeded onto `App` at
  construction + on settings-save. Mirrors the existing `card_density` setting's
  five-touchpoint shape (config → settings UI in `app.rs` → seeded on `App`).
- **Per-view override** stored in an `App` field: `HashMap<SidebarView,
  ViewMode>`. The toolbar toggle writes to this map for the current
  `SidebarView`. A view with no entry falls back to the global default.
- Toolbar control is a 3-segment toggle (☰ List / ▢ Card / ◫ Grid) in the video
  list header.

### 4. Default Dark / Light polish

Promote Dark and Light from stock `egui::Visuals::dark()/light()` to fully
hand-tuned palettes, matching the level of care in the themed variants:

- **Dark** — true near-black panel fill, cool accent (e.g. soft steel-blue),
  tuned widget strokes per state.
- **Light** — warm off-white panel fill (not pure white), slate accent, tuned
  widget strokes per state.

### 5. Typography & spacing refresh

Modest adjustments via egui's built-in proportional font (no new font files):

- Base body text: ~12px → ~13px.
- Introduce hierarchy: distinct sizes/weights for heading / section / card
  title / metadata.
- Standardize card internal padding and inter-row spacing.
- Tighten the metadata line (channel · id · date · duration · size) into a
  consistent rhythm.

### 6. Unify placeholder thumbnails

Replace the two divergent placeholder styles (📺 on gray-30 for channels, ▶ on
gray-38 for videos) with **one consistent style**: a theme-tinted gradient
background (derived from the theme's faint/noninteractive bg) plus a single
subtle glyph, scaled by `card_density`. Used by both channel and video cards.

## Architecture impact

- `theme.rs` — gains 12 theme fns + a `ThemeAccents` struct + a
  `pub fn accents_for(name: &str) -> ThemeAccents` helper.
- `app.rs` — gains `view_mode`/`view_mode_overrides` fields on `App`; the video
  list render path branches on `ViewMode::List/Card/Grid`; all hardcoded accent
  colors read from `self.theme_accents.*`; placeholder paint unified.
- `config.rs` — gains `[ui] default_view_mode` field + `Default` +
  `default_with_dir()`.
- Settings screen (`app.rs`) — gains view-mode selector + theme picker entry
  for the 12 new names.

Follows the documented five-touchpoint settings shape for the new config field.

## Out of scope

- The "Newest" web sort bug — separate work item.
- Web UI visual changes — web has its own CSS and is not touched here.
- New font files / custom typefaces — deferred; uses egui's built-in
  proportional at adjusted sizes.

## Open questions for implementation

None — all design decisions approved during brainstorming:
- 12 themes (3 neon + 3 goth + 3 dev + 3 cozy): approved.
- List/Card/Grid toggle with global default + per-view override: approved.
- Desktop-only scope: approved.
