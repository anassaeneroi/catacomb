# Desktop Visual Refresh + Theme Pack — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add 12 new themes, theme-aware accent colors, a List/Card/Grid view-mode toggle, polished default Dark/Light, refreshed typography, and unified placeholder thumbnails to the catacomb desktop UI.

**Architecture:** All theme logic stays in `theme.rs`; `app.rs` holds a `ThemeAccents` snapshot recomputed whenever the theme changes, plus new `ViewMode`/override state driving three render branches in the video list. A new persisted `[ui] default_view_mode` config field follows the existing `theme`/`ui_scale` shape. Hardcoded accent rings are replaced with reads from `self.theme_accents`.

**Tech Stack:** Rust, egui (eframe), serde/toml. Tests via `cargo test --release`. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-06-27-desktop-visual-refresh-design.md`

**Conventions to follow (from AGENTS.md):**
- `cargo build --release` warnings are the lint.
- The egui dep emits ~39 upstream `f32: From<f64>` warnings on clean build — those are NOT yours.
- Settings five-touchpoint shape: config.rs field + Default → app.rs Settings UI → App field seeded at construction + on save.
- Never commit `config.toml`, `cookies.txt`, `catacomb.db`.

---

## File map

- `src/theme.rs` — add `ThemeAccents` struct + `accents_for()` helper; add 12 theme fns; promote `dark()`/`light()` from stock to hand-tuned; extend `THEMES` catalog to 19.
- `src/config.rs` — add `default_view_mode` field to `UiSection` + Default + serde default fn.
- `src/app.rs` — `ViewMode` enum + `App` fields (`theme_accents`, `view_mode`, `view_mode_overrides`); seed accents at construction; recompute on theme change; branch video list into List/Card/Grid; replace hardcoded accent colors; unify placeholder paint; toolbar 3-segment toggle; typography/spacing bumps; Settings UI for default view mode.
- `tests/api.rs` — no changes (desktop-only, not HTTP).

---

## Task 1: ThemeAccents struct + accents_for() lookup

Establishes the sidecar data model before any theme fns use it.

**Files:**
- Modify: `src/theme.rs:1-24` (top of file, after `use`)

- [ ] **Step 1: Write the failing test**

Append to `src/theme.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accents_for_known_theme_returns_nonzero() {
        let a = accents_for("dark");
        assert_ne!(a.accent, Color32::TRANSPARENT);
        assert_ne!(a.success, Color32::TRANSPARENT);
        assert_ne!(a.warning, Color32::TRANSPARENT);
    }

    #[test]
    fn accents_for_unknown_theme_falls_back() {
        // Unknown theme must still return something usable (falls back to dark).
        let a = accents_for("this-theme-does-not-exist");
        let dark = accents_for("dark");
        assert_eq!(a.accent, dark.accent);
    }

    #[test]
    fn accents_differ_across_themes() {
        // At least two themes should have visibly different accents, proving
        // accents are per-theme, not a global constant.
        let dark = accents_for("dark");
        let light = accents_for("light");
        assert_ne!(dark.accent, light.accent);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --release accents`
Expected: FAIL — `cannot find function accents_for` / `cannot find type ThemeAccents`.

- [ ] **Step 3: Write minimal implementation**

Insert above `pub fn apply` in `src/theme.rs` (after the `use` line at line 1):

```rust
/// Theme-aware semantic accent colors. egui `Visuals` has no slot for these,
/// so each theme exposes them here and the paint code in `app.rs` reads from
/// the active snapshot instead of hardcoding RGB values.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ThemeAccents {
    /// Selection / focus ring (replaces the hardcoded 120,170,230 blue).
    pub accent: Color32,
    /// Playing / watched indicator (replaces 110,200,110 green).
    pub success: Color32,
    /// Bulk-selection highlight (replaces 180,130,240 purple).
    pub warning: Color32,
}

impl ThemeAccents {
    /// The stock fallback used before theme-specific accents are wired in.
    /// Matches the legacy hardcoded values so behavior is unchanged when a
    /// theme has not yet defined its own.
    pub const LEGACY: ThemeAccents = ThemeAccents {
        accent: Color32::from_rgb(120, 170, 230),
        success: Color32::from_rgb(110, 200, 110),
        warning: Color32::from_rgb(180, 130, 240),
    };
}

/// Look up the semantic accents for a theme name. Falls back to the legacy
/// defaults (and dark's accents once dark is hand-tuned) for unknown names.
pub fn accents_for(name: &str) -> ThemeAccents {
    match name {
        "dark" => ThemeAccents { accent: hex(0x7aa2f7), success: hex(0x9ece6a), warning: hex(0xbb9af7) },
        "light" => ThemeAccents { accent: hex(0x2a5db0), success: hex(0x2e7d32), warning: hex(0x8e44ad) },
        "dracula" => ThemeAccents { accent: hex(0xbd93f9), success: hex(0x50fa7b), warning: hex(0xff79c6) },
        "trans" => ThemeAccents { accent: hex(0x55cdfc), success: hex(0x2e7d32), warning: hex(0xf7a8b8) },
        "emo-nocturnal" => ThemeAccents { accent: hex(0xff0090), success: hex(0x39ff14), warning: hex(0x00f5ff) },
        "emo-coffin" => ThemeAccents { accent: hex(0x8b0000), success: hex(0x39ff14), warning: hex(0xcc2222) },
        "emo-scene-queen" => ThemeAccents { accent: hex(0x39ff14), success: hex(0xff00ff), warning: hex(0x00f5ff) },
        // New themes (values match the palettes added in Task 3).
        "cyberpunk" => ThemeAccents { accent: hex(0x00fff5), success: hex(0x39ff14), warning: hex(0xff003c) },
        "synthwave" => ThemeAccents { accent: hex(0xff2a6d), success: hex(0x05d9e8), warning: hex(0xd136a6) },
        "vaporwave" => ThemeAccents { accent: hex(0x01cdfe), success: hex(0x05ffa1), warning: hex(0xff71ce) },
        "cemetery-moss" => ThemeAccents { accent: hex(0x7a8a6a), success: hex(0x4a5d3a), warning: hex(0x9a9a8a) },
        "vampire" => ThemeAccents { accent: hex(0xc9a227), success: hex(0x8b1a2b), warning: hex(0x5c0a1e) },
        "witching-hour" => ThemeAccents { accent: hex(0x6a4a8b), success: hex(0xb0b8d0), warning: hex(0x1a1a4a) },
        "nord" => ThemeAccents { accent: hex(0x88c0d0), success: hex(0xa3be8c), warning: hex(0xebcb8b) },
        "gruvbox" => ThemeAccents { accent: hex(0xfe8019), success: hex(0xb8bb26), warning: hex(0xd3869b) },
        "tokyo-night" => ThemeAccents { accent: hex(0x7aa2f7), success: hex(0x9ece6a), warning: hex(0xbb9af7) },
        "paper" => ThemeAccents { accent: hex(0x8b6f3a), success: hex(0x4a6b3a), warning: hex(0xc4a86a) },
        "honey" => ThemeAccents { accent: hex(0xe8a838), success: hex(0xb87420), warning: hex(0xc97b4a) },
        "candlelight" => ThemeAccents { accent: hex(0xa8703a), success: hex(0x6b3f1e), warning: hex(0xd9b382) },
        _ => ThemeAccents::LEGACY,
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --release accents`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/theme.rs
git commit -m "feat(theme): add ThemeAccents struct + accents_for() lookup"
```

---

## Task 2: Promote default Dark/Light to hand-tuned

The out-of-box themes are currently stock egui. Lift them.

**Files:**
- Modify: `src/theme.rs:13-24` (the `apply` match arms for dark/light)

- [ ] **Step 1: Replace the dark/light match arms with hand-tuned fns**

In `src/theme.rs`, change the `apply` function so its match calls new private fns:

```rust
pub fn apply(ctx: &egui::Context, name: &str) {
    let visuals = match name {
        "light" => light(),
        "dracula" => dracula(),
        "trans" => trans(),
        "emo-nocturnal" => emo_nocturnal(),
        "emo-coffin" => emo_coffin(),
        "emo-scene-queen" => emo_scene_queen(),
        _ => dark(),
    };
    ctx.set_visuals(visuals);
}
```

Then add two new fns (near the other theme fns, before `dracula()`):

```rust
// Hand-tuned default dark — true near-black panel, cool steel-blue accent.
// Replaces the stock egui::Visuals::dark() so the out-of-box experience
// matches the care given to the themed variants.
fn dark() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = hex(0x14141a);
    v.window_fill = hex(0x1a1a22);
    v.extreme_bg_color = hex(0x0a0a0e);
    v.faint_bg_color = hex(0x1f1f29);
    v.code_bg_color = hex(0x101016);
    v.selection.bg_fill = hex(0x2a3a5a);
    v.selection.stroke = Stroke::new(1.0, hex(0x7aa2f7));
    v.hyperlink_color = hex(0x7aa2f7);
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = hex(0x1f1f29);
    v.widgets.noninteractive.weak_bg_fill = hex(0x181820);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0xc8c8d8));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0x2a2a36));
    v.widgets.inactive.bg_fill = hex(0x2a2a36);
    v.widgets.inactive.weak_bg_fill = hex(0x222230);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0xd0d0e0));
    v.widgets.hovered.bg_fill = hex(0x3a3a4e);
    v.widgets.hovered.weak_bg_fill = hex(0x303044);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0xe8e8f8));
    v.widgets.active.bg_fill = hex(0x7aa2f7);
    v.widgets.active.weak_bg_fill = hex(0x5a82d7);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0x14141a));
    v.widgets.open.bg_fill = hex(0x3a3a4e);
    v.window_stroke = Stroke::new(1.0, hex(0x2a2a36));
    v
}

// Hand-tuned default light — warm off-white, slate accent.
fn light() -> egui::Visuals {
    let mut v = egui::Visuals::light();
    v.panel_fill = hex(0xf6f4ef);
    v.window_fill = hex(0xfefcf7);
    v.extreme_bg_color = hex(0xffffff);
    v.faint_bg_color = hex(0xefece4);
    v.code_bg_color = hex(0xeae6dc);
    v.selection.bg_fill = hex(0xbcd0ee);
    v.selection.stroke = Stroke::new(1.0, hex(0x2a5db0));
    v.hyperlink_color = hex(0x2a5db0);
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = hex(0xeae6dc);
    v.widgets.noninteractive.weak_bg_fill = hex(0xf0ede5);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0x3a3a3a));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0xd0ccbf));
    v.widgets.inactive.bg_fill = hex(0xdfe0e8);
    v.widgets.inactive.weak_bg_fill = hex(0xe8e9f0);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0x2a2a2a));
    v.widgets.hovered.bg_fill = hex(0xcdd6e8);
    v.widgets.hovered.weak_bg_fill = hex(0xd8e0ee);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0x141414));
    v.widgets.active.bg_fill = hex(0x2a5db0);
    v.widgets.active.weak_bg_fill = hex(0x4a7dd0);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0xffffff));
    v.widgets.open.bg_fill = hex(0xcdd6e8);
    v.window_stroke = Stroke::new(1.0, hex(0xd0ccbf));
    v
}
```

- [ ] **Step 2: Build to verify it compiles**

Run: `cargo build --release`
Expected: builds clean (ignoring the ~39 upstream `f32: From<f64>` warnings).

- [ ] **Step 3: Smoke-test visually**

Run: `./target/release/catacomb`
Open Settings, switch Dark ↔ Light. Confirm panels/panels/widgets look distinct from the previous stock egui look (near-black panel for dark, warm off-white for light).

- [ ] **Step 4: Commit**

```bash
git add src/theme.rs
git commit -m "feat(theme): hand-tune default Dark/Light themes"
```

---

## Task 3: Add 12 new theme functions + catalog

**Files:**
- Modify: `src/theme.rs:3-11` (THEMES catalog) and append 12 fns at end of file

- [ ] **Step 1: Extend the THEMES catalog**

Replace the `THEMES` const at `src/theme.rs:3-11`:

```rust
pub const THEMES: &[(&str, &str)] = &[
    ("dark", "Dark"),
    ("light", "Light"),
    ("dracula", "Dracula"),
    ("trans", "Trans"),
    // Catacomb / goth
    ("emo-nocturnal", "Emo: Nocturnal"),
    ("emo-coffin", "Emo: Coffin"),
    ("emo-scene-queen", "Emo: Scene Queen"),
    ("cemetery-moss", "Cemetery Moss"),
    ("vampire", "Vampire"),
    ("witching-hour", "Witching Hour"),
    // Neon / retro
    ("cyberpunk", "Cyberpunk"),
    ("synthwave", "Synthwave '84"),
    ("vaporwave", "Vaporwave"),
    // Dev palettes
    ("nord", "Nord"),
    ("gruvbox", "Gruvbox"),
    ("tokyo-night", "Tokyo Night"),
    // Cozy / light
    ("paper", "Paper"),
    ("honey", "Honey"),
    ("candlelight", "Candlelight"),
];
```

- [ ] **Step 2: Wire the new theme names into `apply`**

Update the `apply` match in `src/theme.rs` to dispatch the new names:

```rust
pub fn apply(ctx: &egui::Context, name: &str) {
    let visuals = match name {
        "light" => light(),
        "dracula" => dracula(),
        "trans" => trans(),
        "emo-nocturnal" => emo_nocturnal(),
        "emo-coffin" => emo_coffin(),
        "emo-scene-queen" => emo_scene_queen(),
        "cemetery-moss" => cemetery_moss(),
        "vampire" => vampire(),
        "witching-hour" => witching_hour(),
        "cyberpunk" => cyberpunk(),
        "synthwave" => synthwave(),
        "vaporwave" => vaporwave(),
        "nord" => nord(),
        "gruvbox" => gruvbox(),
        "tokyo-night" => tokyo_night(),
        "paper" => paper(),
        "honey" => honey(),
        "candlelight" => candlelight(),
        _ => dark(),
    };
    ctx.set_visuals(visuals);
}
```

- [ ] **Step 3: Append the 12 theme functions**

Append to the end of `src/theme.rs`:

```rust
// === Neon / retro ===

// Magenta + electric cyan on black. Hacker HUD.
fn cyberpunk() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = hex(0x0a0a12);
    v.window_fill = hex(0x0e0e18);
    v.extreme_bg_color = hex(0x050508);
    v.faint_bg_color = hex(0x14141f);
    v.code_bg_color = hex(0x08080f);
    v.selection.bg_fill = hex(0xff003c);
    v.selection.stroke = Stroke::new(1.0, hex(0x00fff5));
    v.hyperlink_color = hex(0x00fff5);
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = hex(0x14141f);
    v.widgets.noninteractive.weak_bg_fill = hex(0x0f0f18);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0xc8c8e0));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0x2a2a3a));
    v.widgets.inactive.bg_fill = hex(0x1f0018);
    v.widgets.inactive.weak_bg_fill = hex(0x180013);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0xd0d0e8));
    v.widgets.hovered.bg_fill = hex(0x6a0044);
    v.widgets.hovered.weak_bg_fill = hex(0x500034);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0x00fff5));
    v.widgets.active.bg_fill = hex(0xff003c);
    v.widgets.active.weak_bg_fill = hex(0xcc0030);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0xffffff));
    v.widgets.open.bg_fill = hex(0x6a0044);
    v.window_stroke = Stroke::new(1.0, hex(0xff003c));
    v.warn_fg_color = hex(0xfcee0a);
    v.error_fg_color = hex(0xff003c);
    v
}

// Sunset gradient on deep indigo.
fn synthwave() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = hex(0x2b0a3d);
    v.window_fill = hex(0x330a48);
    v.extreme_bg_color = hex(0x1a0529);
    v.faint_bg_color = hex(0x3a1055);
    v.code_bg_color = hex(0x220833);
    v.selection.bg_fill = hex(0xff2a6d);
    v.selection.stroke = Stroke::new(1.0, hex(0x05d9e8));
    v.hyperlink_color = hex(0x05d9e8);
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = hex(0x3a1055);
    v.widgets.noninteractive.weak_bg_fill = hex(0x2e0c45);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0xe8c0e0));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0x5a1a7a));
    v.widgets.inactive.bg_fill = hex(0x4a1466);
    v.widgets.inactive.weak_bg_fill = hex(0x3c1055);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0xf0d0e8));
    v.widgets.hovered.bg_fill = hex(0x7a1a4a);
    v.widgets.hovered.weak_bg_fill = hex(0x60143a);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0xff9a3c));
    v.widgets.active.bg_fill = hex(0xff2a6d);
    v.widgets.active.weak_bg_fill = hex(0xcc2055);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0xffffff));
    v.widgets.open.bg_fill = hex(0x7a1a4a);
    v.window_stroke = Stroke::new(1.0, hex(0xff2a6d));
    v.warn_fg_color = hex(0xff9a3c);
    v.error_fg_color = hex(0xff2a6d);
    v
}

// Pastel pink + cyan + lavender on deep plum.
fn vaporwave() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = hex(0x1a0033);
    v.window_fill = hex(0x200040);
    v.extreme_bg_color = hex(0x10001f);
    v.faint_bg_color = hex(0x28004e);
    v.code_bg_color = hex(0x150028);
    v.selection.bg_fill = hex(0xb967ff);
    v.selection.stroke = Stroke::new(1.0, hex(0x01cdfe));
    v.hyperlink_color = hex(0x01cdfe);
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = hex(0x28004e);
    v.widgets.noninteractive.weak_bg_fill = hex(0x1f003c);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0xe0c0ff));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0x4a0070));
    v.widgets.inactive.bg_fill = hex(0x350060);
    v.widgets.inactive.weak_bg_fill = hex(0x2a004f);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0xf0d8ff));
    v.widgets.hovered.bg_fill = hex(0x55008a);
    v.widgets.hovered.weak_bg_fill = hex(0x440070);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0x01cdfe));
    v.widgets.active.bg_fill = hex(0xff71ce);
    v.widgets.active.weak_bg_fill = hex(0xcc5aa6);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0x1a0033));
    v.widgets.open.bg_fill = hex(0x55008a);
    v.window_stroke = Stroke::new(1.0, hex(0xff71ce));
    v.warn_fg_color = hex(0x05ffa1);
    v.error_fg_color = hex(0xff71ce);
    v
}

// === Catacomb / goth ===

// Weathered stone + mossy green + bone.
fn cemetery_moss() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = hex(0x1a1f1a);
    v.window_fill = hex(0x1e241e);
    v.extreme_bg_color = hex(0x0f140f);
    v.faint_bg_color = hex(0x242a24);
    v.code_bg_color = hex(0x141814);
    v.selection.bg_fill = hex(0x4a5d3a);
    v.selection.stroke = Stroke::new(1.0, hex(0x7a8a6a));
    v.hyperlink_color = hex(0x9aa86a);
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = hex(0x242a24);
    v.widgets.noninteractive.weak_bg_fill = hex(0x1f241f);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0xb8b8a8));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0x363a30));
    v.widgets.inactive.bg_fill = hex(0x2a302a);
    v.widgets.inactive.weak_bg_fill = hex(0x232823);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0xc8c8b8));
    v.widgets.hovered.bg_fill = hex(0x3a4530);
    v.widgets.hovered.weak_bg_fill = hex(0x2e3826);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0xaac08a));
    v.widgets.active.bg_fill = hex(0x4a5d3a);
    v.widgets.active.weak_bg_fill = hex(0x3a4a2e);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0xd4d0c0));
    v.widgets.open.bg_fill = hex(0x3a4530);
    v.window_stroke = Stroke::new(1.0, hex(0x4a5d3a));
    v.warn_fg_color = hex(0xc9a227);
    v.error_fg_color = hex(0x8b1a2b);
    v
}

// Deep wine burgundy + antique gold + black.
fn vampire() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = hex(0x0d0006);
    v.window_fill = hex(0x120008);
    v.extreme_bg_color = hex(0x070003);
    v.faint_bg_color = hex(0x18000c);
    v.code_bg_color = hex(0x0a0005);
    v.selection.bg_fill = hex(0x5c0a1e);
    v.selection.stroke = Stroke::new(1.0, hex(0xc9a227));
    v.hyperlink_color = hex(0xc9a227);
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = hex(0x18000c);
    v.widgets.noninteractive.weak_bg_fill = hex(0x130009);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0xd8c8a8));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0x3a0014));
    v.widgets.inactive.bg_fill = hex(0x200010);
    v.widgets.inactive.weak_bg_fill = hex(0x19000c);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0xe8d8b8));
    v.widgets.hovered.bg_fill = hex(0x400018);
    v.widgets.hovered.weak_bg_fill = hex(0x300012);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0xc9a227));
    v.widgets.active.bg_fill = hex(0x8b1a2b);
    v.widgets.active.weak_bg_fill = hex(0x700020);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0xe8d8b8));
    v.widgets.open.bg_fill = hex(0x400018);
    v.window_stroke = Stroke::new(1.0, hex(0x8b1a2b));
    v.warn_fg_color = hex(0xc9a227);
    v.error_fg_color = hex(0x8b1a2b);
    v
}

// Midnight indigo + moonlight silver + arcane violet.
fn witching_hour() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = hex(0x0a0a1f);
    v.window_fill = hex(0x0e0e28);
    v.extreme_bg_color = hex(0x05050f);
    v.faint_bg_color = hex(0x14142e);
    v.code_bg_color = hex(0x08081a);
    v.selection.bg_fill = hex(0x1a1a4a);
    v.selection.stroke = Stroke::new(1.0, hex(0xb0b8d0));
    v.hyperlink_color = hex(0xb0b8d0);
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = hex(0x14142e);
    v.widgets.noninteractive.weak_bg_fill = hex(0x101024);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0xc0c8e0));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0x2a2a55));
    v.widgets.inactive.bg_fill = hex(0x1c1c3c);
    v.widgets.inactive.weak_bg_fill = hex(0x161630);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0xd0d8f0));
    v.widgets.hovered.bg_fill = hex(0x2a2a5a);
    v.widgets.hovered.weak_bg_fill = hex(0x20204a);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0x6a4a8b));
    v.widgets.active.bg_fill = hex(0x6a4a8b);
    v.widgets.active.weak_bg_fill = hex(0x523a72);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0xe8e8f8));
    v.widgets.open.bg_fill = hex(0x2a2a5a);
    v.window_stroke = Stroke::new(1.0, hex(0x6a4a8b));
    v.warn_fg_color = hex(0xc9a227);
    v.error_fg_color = hex(0x8b1a2b);
    v
}

// === Dev palettes ===

// Arctic blues & greys.
fn nord() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = hex(0x2e3440);
    v.window_fill = hex(0x3b4252);
    v.extreme_bg_color = hex(0x1e222a);
    v.faint_bg_color = hex(0x3b4252);
    v.code_bg_color = hex(0x242933);
    v.selection.bg_fill = hex(0x434c5e);
    v.selection.stroke = Stroke::new(1.0, hex(0x88c0d0));
    v.hyperlink_color = hex(0x88c0d0);
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = hex(0x3b4252);
    v.widgets.noninteractive.weak_bg_fill = hex(0x343b48);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0xd8dee9));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0x434c5e));
    v.widgets.inactive.bg_fill = hex(0x434c5e);
    v.widgets.inactive.weak_bg_fill = hex(0x3c4454);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0xe5e9f0));
    v.widgets.hovered.bg_fill = hex(0x4c566a);
    v.widgets.hovered.weak_bg_fill = hex(0x424c60);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0x88c0d0));
    v.widgets.active.bg_fill = hex(0x88c0d0);
    v.widgets.active.weak_bg_fill = hex(0x6fa6ba);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0x2e3440));
    v.widgets.open.bg_fill = hex(0x4c566a);
    v.window_stroke = Stroke::new(1.0, hex(0x4c566a));
    v.warn_fg_color = hex(0xebcb8b);
    v.error_fg_color = hex(0xbf616a);
    v
}

// Warm earthy retro groove.
fn gruvbox() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = hex(0x282828);
    v.window_fill = hex(0x32302f);
    v.extreme_bg_color = hex(0x1d2021);
    v.faint_bg_color = hex(0x32302f);
    v.code_bg_color = hex(0x1d2021);
    v.selection.bg_fill = hex(0x504945);
    v.selection.stroke = Stroke::new(1.0, hex(0xfe8019));
    v.hyperlink_color = hex(0x83a598);
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = hex(0x32302f);
    v.widgets.noninteractive.weak_bg_fill = hex(0x2c2a29);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0xd5c4a1));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0x504945));
    v.widgets.inactive.bg_fill = hex(0x504945);
    v.widgets.inactive.weak_bg_fill = hex(0x44403c);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0xebdbb2));
    v.widgets.hovered.bg_fill = hex(0x665c54);
    v.widgets.hovered.weak_bg_fill = hex(0x585048);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0xfe8019));
    v.widgets.active.bg_fill = hex(0xd65d0e);
    v.widgets.active.weak_bg_fill = hex(0xaf4f0b);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0xebdbb2));
    v.widgets.open.bg_fill = hex(0x665c54);
    v.window_stroke = Stroke::new(1.0, hex(0x665c54));
    v.warn_fg_color = hex(0xfe9d44);
    v.error_fg_color = hex(0xfb4934);
    v
}

// Tokyo city lights. Blue/purple, clean.
fn tokyo_night() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = hex(0x1a1b26);
    v.window_fill = hex(0x20202f);
    v.extreme_bg_color = hex(0x12121c);
    v.faint_bg_color = hex(0x24253a);
    v.code_bg_color = hex(0x16161e);
    v.selection.bg_fill = hex(0x33415c);
    v.selection.stroke = Stroke::new(1.0, hex(0x7aa2f7));
    v.hyperlink_color = hex(0x7aa2f7);
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = hex(0x24253a);
    v.widgets.noninteractive.weak_bg_fill = hex(0x1f2030);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0xa9b1d6));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0x3b4261));
    v.widgets.inactive.bg_fill = hex(0x2a2b3e);
    v.widgets.inactive.weak_bg_fill = hex(0x232436);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0xc0caf5));
    v.widgets.hovered.bg_fill = hex(0x363b54);
    v.widgets.hovered.weak_bg_fill = hex(0x2d324a);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0x7aa2f7));
    v.widgets.active.bg_fill = hex(0x7aa2f7);
    v.widgets.active.weak_bg_fill = hex(0x5c82d4);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0x1a1b26));
    v.widgets.open.bg_fill = hex(0x363b54);
    v.window_stroke = Stroke::new(1.0, hex(0x3b4261));
    v.warn_fg_color = hex(0xe0af68);
    v.error_fg_color = hex(0xf7768e);
    v
}

// === Cozy / light ===

// Aged paper + sepia ink.
fn paper() -> egui::Visuals {
    let mut v = egui::Visuals::light();
    v.panel_fill = hex(0xf4ecd8);
    v.window_fill = hex(0xf8f0dc);
    v.extreme_bg_color = hex(0xfef8e8);
    v.faint_bg_color = hex(0xefe5cc);
    v.code_bg_color = hex(0xe8dcc0);
    v.selection.bg_fill = hex(0xd9c896);
    v.selection.stroke = Stroke::new(1.0, hex(0x8b6f3a));
    v.hyperlink_color = hex(0x6b4f2a);
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = hex(0xefe5cc);
    v.widgets.noninteractive.weak_bg_fill = hex(0xf2e9d2);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0x3d2b1f));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0xc4a86a));
    v.widgets.inactive.bg_fill = hex(0xe2d4b0);
    v.widgets.inactive.weak_bg_fill = hex(0xeadab4);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0x2d1f15));
    v.widgets.hovered.bg_fill = hex(0xd6c498);
    v.widgets.hovered.weak_bg_fill = hex(0xddcaa8);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0x3d2b1f));
    v.widgets.active.bg_fill = hex(0x8b6f3a);
    v.widgets.active.weak_bg_fill = hex(0xa18450);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0xf4ecd8));
    v.widgets.open.bg_fill = hex(0xd6c498);
    v.window_stroke = Stroke::new(1.0, hex(0xc4a86a));
    v.warn_fg_color = hex(0xb87420);
    v.error_fg_color = hex(0x9a3a1a);
    v
}

// Warm amber + gold + cream.
fn honey() -> egui::Visuals {
    let mut v = egui::Visuals::light();
    v.panel_fill = hex(0xfff4e0);
    v.window_fill = hex(0xfff9ec);
    v.extreme_bg_color = hex(0xffffff);
    v.faint_bg_color = hex(0xffecc8);
    v.code_bg_color = hex(0xffe6b8);
    v.selection.bg_fill = hex(0xffd97a);
    v.selection.stroke = Stroke::new(1.0, hex(0xb87420));
    v.hyperlink_color = hex(0x9a5a10);
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = hex(0xffecc8);
    v.widgets.noninteractive.weak_bg_fill = hex(0xfff2d4);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0x5c3818));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0xe8a838));
    v.widgets.inactive.bg_fill = hex(0xffdf9c);
    v.widgets.inactive.weak_bg_fill = hex(0xffe6b0);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0x4a2c10));
    v.widgets.hovered.bg_fill = hex(0xffd166);
    v.widgets.hovered.weak_bg_fill = hex(0xffdb84);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0x4a2c10));
    v.widgets.active.bg_fill = hex(0xe8a838);
    v.widgets.active.weak_bg_fill = hex(0xc8902a);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0xfff4e0));
    v.widgets.open.bg_fill = hex(0xffd166);
    v.window_stroke = Stroke::new(1.0, hex(0xe8a838));
    v.warn_fg_color = hex(0xb87420);
    v.error_fg_color = hex(0xa83a1a);
    v
}

// Dim warm glow + toasted brown.
fn candlelight() -> egui::Visuals {
    let mut v = egui::Visuals::light();
    v.panel_fill = hex(0xf2e6d0);
    v.window_fill = hex(0xf6ecd9);
    v.extreme_bg_color = hex(0xfbf3e2);
    v.faint_bg_color = hex(0xead8b8);
    v.code_bg_color = hex(0xe2cea0);
    v.selection.bg_fill = hex(0xd9b382);
    v.selection.stroke = Stroke::new(1.0, hex(0x6b3f1e));
    v.hyperlink_color = hex(0x5a3418);
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = hex(0xead8b8);
    v.widgets.noninteractive.weak_bg_fill = hex(0xeedec2);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0x3a2818));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0xa8703a));
    v.widgets.inactive.bg_fill = hex(0xddc498);
    v.widgets.inactive.weak_bg_fill = hex(0xe4ceaa);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0x2d1a0c));
    v.widgets.hovered.bg_fill = hex(0xd2ac6e);
    v.widgets.hovered.weak_bg_fill = hex(0xdab880);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0x2d1a0c));
    v.widgets.active.bg_fill = hex(0xa8703a);
    v.widgets.active.weak_bg_fill = hex(0x8c5a2c);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0xf2e6d0));
    v.widgets.open.bg_fill = hex(0xd2ac6e);
    v.window_stroke = Stroke::new(1.0, hex(0xa8703a));
    v.warn_fg_color = hex(0xb87420);
    v.error_fg_color = hex(0x9a3a1a);
    v
}
```

- [ ] **Step 4: Build to verify it compiles**

Run: `cargo build --release`
Expected: builds clean (ignoring the ~39 upstream `f32: From<f64>` warnings). If any hex literal errors appear, double-check the values against the palette comments.

- [ ] **Step 5: Smoke-test all 19 themes**

Run: `./target/release/catacomb`
Open Settings → Theme, click through all 19. Confirm each renders without panic and looks like its identity.

- [ ] **Step 6: Commit**

```bash
git add src/theme.rs
git commit -m "feat(theme): add 12 new themes (cyberpunk, goth, dev, cozy)"
```

---

## Task 4: Add `default_view_mode` to config

**Files:**
- Modify: `src/config.rs:190-222` (UiSection + Default) and add a default fn near line 279

- [ ] **Step 1: Write the failing test**

Add a test module at the end of `src/config.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_section_default_view_mode_is_list() {
        let ui = UiSection::default();
        assert_eq!(ui.default_view_mode, "list");
    }

    #[test]
    fn ui_section_round_trips_through_toml() {
        let ui = UiSection { default_view_mode: "grid".into(), ..Default::default() };
        let s = toml::to_string(&ui).unwrap();
        let back: UiSection = toml::from_str(&s).unwrap();
        assert_eq!(back.default_view_mode, "grid");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --release config::tests`
Expected: FAIL — no field `default_view_mode`.

- [ ] **Step 3: Add the field + Default + default fn**

In `src/config.rs`, add the field to `UiSection` (after `ui_scale`):

```rust
    /// Default video-list render mode: "list", "card", or "grid".
    /// Per-view overrides live in App state (not persisted beyond session).
    #[serde(default = "default_view_mode")]
    pub default_view_mode: String,
```

Add the default fn near `default_theme()` (line ~279):

```rust
fn default_view_mode() -> String { "list".to_string() }
```

Update `impl Default for UiSection`:

```rust
impl Default for UiSection {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            minimize_to_tray: false,
            ui_scale: default_ui_scale(),
            default_view_mode: default_view_mode(),
        }
    }
}
```

Update the doc comment listing themes at line 192 if it enumerates them — leave the theme list as-is (it's still accurate), just note view modes are documented elsewhere. (Optional: no change required.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --release config::tests`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat(config): add [ui] default_view_mode field"
```

---

## Task 5: Wire ThemeAccents into App + recompute on theme change

**Files:**
- Modify: `src/app.rs:167` (App fields), `:371` (construction), `:3178` (theme picker callback)

- [ ] **Step 1: Add the App field**

In `src/app.rs`, add to the `App` struct fields near `card_density` (line 167):

```rust
    /// Theme-aware accent colors, recomputed whenever the theme changes.
    theme_accents: crate::theme::ThemeAccents,
```

- [ ] **Step 2: Seed at construction**

At `src/app.rs:371` where `theme::apply(&cc.egui_ctx, &config.ui.theme);` is called, add immediately after:

```rust
        theme::apply(&cc.egui_ctx, &config.ui.theme);
        let theme_accents = theme::accents_for(&config.ui.theme);
```

Then in the `App { ... }` literal (around line 553, near `card_density: 1.0,`), add:

```rust
            theme_accents,
```

- [ ] **Step 3: Recompute on theme change**

In the Settings theme picker callback at `src/app.rs:3178`, change:

```rust
                                    {
                                        self.config.ui.theme = id.to_string();
                                        theme::apply(ctx, id);
                                    }
```

to:

```rust
                                    {
                                        self.config.ui.theme = id.to_string();
                                        theme::apply(ctx, id);
                                        self.theme_accents = theme::accents_for(id);
                                    }
```

- [ ] **Step 4: Build to verify**

Run: `cargo build --release`
Expected: builds clean.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): wire ThemeAccents snapshot, recompute on theme change"
```

---

## Task 6: Replace hardcoded accent colors with theme accents

**Files:**
- Modify: `src/app.rs:4225, 4232, 4239` (the three hardcoded rings in the video list)

- [ ] **Step 1: Replace the three hardcoded colors**

At `src/app.rs:4225` change:
```rust
                            egui::Stroke::new(2.0, egui::Color32::from_rgb(120, 170, 230)),
```
to:
```rust
                            egui::Stroke::new(2.0, self.theme_accents.accent),
```

At `src/app.rs:4232` change:
```rust
                            egui::Stroke::new(2.0, egui::Color32::from_rgb(110, 200, 110)),
```
to:
```rust
                            egui::Stroke::new(2.0, self.theme_accents.success),
```

At `src/app.rs:4239` change:
```rust
                            egui::Stroke::new(3.0, egui::Color32::from_rgb(180, 130, 240)),
```
to:
```rust
                            egui::Stroke::new(3.0, self.theme_accents.warning),
```

- [ ] **Step 2: Build + smoke-test**

Run: `cargo build --release && ./target/release/catacomb`
Switch themes in Settings; select/play/bulk-check videos. Confirm the rings change color to match each theme (e.g. Witching Hour = arcane-violet, Honey = amber).

- [ ] **Step 3: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): use theme-aware accent rings for select/play/bulk"
```

---

## Task 7: ViewMode enum + App state + global default seeding

**Files:**
- Modify: `src/app.rs` (new enum near `SortMode` at line 66; App fields near line 167)

- [ ] **Step 1: Add the ViewMode enum**

Near the `SortMode` enum at `src/app.rs:66`, add:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ViewMode {
    List,
    Card,
    Grid,
}

impl ViewMode {
    fn from_config(s: &str) -> Self {
        match s {
            "card" => ViewMode::Card,
            "grid" => ViewMode::Grid,
            _ => ViewMode::List,
        }
    }
}
```

- [ ] **Step 2: Add App fields**

Near `card_density` in the `App` struct (line 167), add:

```rust
    /// Global default video-list render mode (from config).
    default_view_mode: ViewMode,
    /// Per-SidebarView overrides; a view absent here falls back to default.
    view_mode_overrides: std::collections::HashMap<SidebarView, ViewMode>,
```

- [ ] **Step 3: Seed at construction**

In the `App { ... }` literal near `card_density: 1.0,` (line 553), add:

```rust
            default_view_mode: ViewMode::from_config(&config.ui.default_view_mode),
            view_mode_overrides: Default::default(),
```

- [ ] **Step 4: Add a helper to resolve the active mode for a view**

Add an `impl App` method (place near other small helpers in the same impl block):

```rust
    /// The view mode to use for `view`: the per-view override if set, else
    /// the global default.
    fn view_mode_for(&self, view: &SidebarView) -> ViewMode {
        self.view_mode_overrides
            .get(view)
            .copied()
            .unwrap_or(self.default_view_mode)
    }
```

- [ ] **Step 5: Build to verify**

Run: `cargo build --release`
Note: `SidebarView` must derive `Hash` and `Eq` for HashMap use. Check the derive at `src/app.rs:79` (`#[derive(Clone, PartialEq)]`) and add `Eq, Hash`:

```rust
#[derive(Clone, PartialEq, Eq, Hash)]
enum SidebarView {
```
(Verify all variants are compatible — they are plain enums/struct tuples of `usize`, which all impl Eq+Hash.)

Expected: builds clean.

- [ ] **Step 6: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): ViewMode enum + global default + per-view overrides"
```

---

## Task 8: Unify placeholder thumbnail paint

**Files:**
- Modify: `src/app.rs:3972-3979` (channel card placeholder) and `src/app.rs:4210-4217` (video card placeholder)

- [ ] **Step 1: Add a shared placeholder helper**

Add an `impl App` method:

```rust
    /// Paint a consistent missing-thumbnail placeholder inside `rect`:
    /// a theme-tinted gradient + a single glyph. Used for both channel and
    /// video cards so the empty states stop diverging.
    fn paint_thumb_placeholder(&self, ui: &egui::Ui, rect: egui::Rect, glyph: &str, density: f32) {
        let v = ui.visuals();
        // Two-tone vertical gradient from faint_bg to panel_fill.
        let top = v.faint_bg_color;
        let bot = v.panel_fill;
        let (top, bot) = (top.to_array(), bot.to_array());
        // Use a simple split fill: top half one color, bottom half another,
        // blended by drawing two stacked rects (egui has no native gradient).
        let mid = rect.top() + rect.height() * 0.5;
        ui.painter().rect_filled(
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, mid)),
            4.0,
            egui::Color32::from_rgba_unmultiplied(top[0], top[1], top[2], 255),
        );
        ui.painter().rect_filled(
            egui::Rect::from_min_max(egui::pos2(rect.min.x, mid), rect.max),
            4.0,
            egui::Color32::from_rgba_unmultiplied(bot[0], bot[1], bot[2], 255),
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            egui::FontId::proportional(24.0 * density),
            v.weak_text_color(),
        );
    }
```

- [ ] **Step 2: Replace the channel-card placeholder**

At `src/app.rs:3972-3980`, replace the `None => { ... }` arm with:

```rust
                                None => {
                                    self.paint_thumb_placeholder(ui, thumb_rect, "🎬", density);
                                }
```

- [ ] **Step 3: Replace the video-card placeholder**

At `src/app.rs:4210-4218`, replace the `None => { ... }` arm with:

```rust
                        None => {
                            self.paint_thumb_placeholder(ui, rect, "🎬", density);
                        }
```

- [ ] **Step 4: Build + smoke-test**

Run: `cargo build --release && ./target/release/catacomb`
Find a video/channel with no thumbnail. Confirm both now render the same theme-tinted gradient + 🎬 glyph.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): unified theme-tinted placeholder thumbnails"
```

---

## Task 9: List / Card / Grid render branches in the video list

This is the largest task. The existing flat-row code becomes the `List` branch; `Card` wraps each row in a rounded rect; `Grid` reflows into vertical cards.

**Files:**
- Modify: `src/app.rs:4184-4014` (the `for card in &cards { ... }` loop inside the `ScrollArea`)

- [ ] **Step 1: Read the current loop fully**

Run: read `src/app.rs` lines 4184–4014 to see the existing `ui.horizontal(|ui| { ... })` row body. The title/metadata/flags body must be reused across all three modes.

- [ ] **Step 2: Wrap the existing row code in a `match self.view_mode_for(&self.sidebar_view.clone())`**

At the top of the `ScrollArea::vertical().show(ui, |ui| { ... })` closure (line ~4184), after the `thumb_w/thumb_h` setup, insert the mode resolution:

```rust
            let view_mode = self.view_mode_for(&self.sidebar_view.clone());
```

(We clone `sidebar_view` to avoid borrowing `self` while the loop also borrows `cards`/`self` mutably via flag writes — match the existing pattern where the loop body uses `let mut clicked_card = false;` etc. to defer mutations.)

Then branch. The simplest refactor that preserves all existing behavior for `List`:

```rust
            match view_mode {
                ViewMode::List => {
                    // === existing loop body, unchanged ===
                    for card in &cards {
                        // ... (the entire existing row code stays here verbatim)
                    }
                }
                ViewMode::Card => {
                    self.render_video_list_cards(ui, ctx, &cards, density);
                }
                ViewMode::Grid => {
                    self.render_video_list_grid(ui, ctx, &cards, density);
                }
            }
```

Move the entire existing `for card in &cards { ... }` block under the `ViewMode::List` arm (cut and paste verbatim — no changes to its internals yet).

- [ ] **Step 3: Add the `Card` render helper**

Add `impl App` methods (signatures must match how they're called above):

```rust
    /// Card-row mode: same horizontal layout as List, but each row is a
    /// rounded card on faint_bg_fill with a hover lift.
    fn render_video_list_cards(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        cards: &[VideoCard],
        density: f32,
    ) {
        let thumb_w = (176.0 * density).round();
        let thumb_h = (99.0 * density).round();
        let thumb_size = egui::vec2(thumb_w, thumb_h);

        for (i, card) in cards.iter().enumerate() {
            // Per-row mutations are deferred with the same flag-out pattern
            // the List branch uses; mirror it exactly.
            let mut clicked_card = false;
            let mut play_card = false;
            let mut toggle_watched_card = false;
            let mut toggle_flag_card: Option<&'static str> = None;
            let selected = self.selected_video.as_deref() == Some(card.id.as_str());
            let is_playing = self.currently_playing.as_deref() == Some(card.id.as_str());
            let bulk_checked = self.bulk_selected.contains(&card.id);

            let frame = egui::Frame::NONE
                .fill(ui.visuals().faint_bg_color)
                .stroke(egui::Stroke::new(
                    1.0,
                    ui.visuals().widgets.noninteractive.bg_stroke.color,
                ))
                .rounding(egui::Rounding::same(8.0))
                .inner_margin(egui::Margin::same(8))
                .outer_margin(egui::Margin::symmetric(0, 3));

            let card_resp = frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (rect, resp) = ui.allocate_exact_size(thumb_size, egui::Sense::click());
                    let texture = card.thumb_path.as_ref().and_then(|p| self.texture(ctx, p));
                    match &texture {
                        Some(handle) => {
                            egui::Image::new(handle).maintain_aspect_ratio(true).paint_at(ui, rect);
                        }
                        None => self.paint_thumb_placeholder(ui, rect, "🎬", density),
                    }
                    if selected {
                        ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(2.0, self.theme_accents.accent));
                    }
                    if is_playing {
                        ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(2.0, self.theme_accents.success));
                    }
                    if bulk_checked {
                        ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(3.0, self.theme_accents.warning));
                    }
                    if resp.clicked() { clicked_card = true; }
                    if resp.double_clicked() { play_card = true; }
                    self.render_video_meta_row(ui, card, selected, &mut clicked_card);
                });
            });
            if card_resp.response.hovered() {
                ui.painter().rect_stroke(
                    card_resp.response.rect,
                    8.0,
                    egui::Stroke::new(1.5, self.theme_accents.accent),
                );
            }
            self.apply_video_card_actions(
                i, cards, clicked_card, play_card, toggle_watched_card, toggle_flag_card,
            );
            ui.add_space(4.0);
        }
    }
```

- [ ] **Step 4: Add the `Grid` render helper**

```rust
    /// Grid mode: YouTube/Plex-style vertical cards, responsive columns.
    fn render_video_list_grid(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        cards: &[VideoCard],
        density: f32,
    ) {
        let thumb_w = (176.0 * density).round();
        let thumb_h = (99.0 * density).round();
        let card_w = thumb_w + 8.0;
        let avail = ui.available_width();
        let cols = ((avail / card_w).floor() as usize).max(1);

        let mut clicked_card = false;
        let mut play_card = false;
        let mut toggle_watched_card = false;
        let mut toggle_flag_card: Option<&'static str> = None;

        egui::Grid::new("video_grid")
            .num_columns(cols)
            .spacing([8.0, 8.0])
            .show(ui, |ui| {
                for (i, card) in cards.iter().enumerate() {
                    let selected = self.selected_video.as_deref() == Some(card.id.as_str());
                    let is_playing = self.currently_playing.as_deref() == Some(card.id.as_str());
                    let bulk_checked = self.bulk_selected.contains(&card.id);

                    let (rect, resp) = ui.allocate_exact_size(
                        egui::vec2(thumb_w, thumb_h),
                        egui::Sense::click(),
                    );
                    let texture = card.thumb_path.as_ref().and_then(|p| self.texture(ctx, p));
                    match &texture {
                        Some(handle) => {
                            egui::Image::new(handle).maintain_aspect_ratio(true).paint_at(ui, rect);
                        }
                        None => self.paint_thumb_placeholder(ui, rect, "🎬", density),
                    }
                    if selected {
                        ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(2.0, self.theme_accents.accent));
                    }
                    if is_playing {
                        ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(2.0, self.theme_accents.success));
                    }
                    if bulk_checked {
                        ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(3.0, self.theme_accents.warning));
                    }
                    if resp.clicked() { clicked_card = true; }
                    if resp.double_clicked() { play_card = true; }

                    ui.vertical(|ui| {
                        ui.add_space(4.0);
                        self.render_video_meta_row(ui, card, selected, &mut clicked_card);
                    });
                    ui.end_row();
                    let _ = i;
                }
            });
        self.apply_video_card_actions(
            0, cards, clicked_card, play_card, toggle_watched_card, toggle_flag_card,
        );
    }
```

- [ ] **Step 5: Extract the shared metadata/title/flags body into `render_video_meta_row` and the action application into `apply_video_card_actions`**

These two helpers are refactor extractions from the existing `List` branch. **Read the existing row body at `src/app.rs:4267-4400` (the `ui.vertical(|ui| { ... })` block containing title, channel, id, duration, size, flag buttons) and move it verbatim into:**

```rust
    /// Shared title + metadata + flag-button row, used by all three view modes.
    fn render_video_meta_row(
        &mut self,
        ui: &mut egui::Ui,
        card: &VideoCard,
        selected: bool,
        clicked_card: &mut bool,
    ) {
        // === the body of the existing ui.vertical(|ui| { ... }) block ===
        // (title selectable_label, channel/id/duration/size metadata, flag
        //  buttons). Copy verbatim from the current List branch.
    }
```

And the deferred-mutation application (the code after each row that writes `self.flags`, `self.selected_video`, plays, etc.) into:

```rust
    /// Apply the per-card deferred actions (click/play/watch/flag toggles).
    /// `cards` + `i` let it locate the right card for DB writes.
    fn apply_video_card_actions(
        &mut self,
        i: usize,
        cards: &[VideoCard],
        clicked_card: bool,
        play_card: bool,
        toggle_watched_card: bool,
        toggle_flag_card: Option<&'static str>,
    ) {
        // === the existing post-row mutation code, factored out ===
        // (the `if clicked_card { self.selected_video = ... }` etc. block)
    }
```

**Important:** because the exact contents of these blocks depend on the current code (which may have evolved), the engineer implementing this MUST read `src/app.rs:4267-4420` and copy the real code rather than trusting the comments above. The signatures above are the contract; the bodies are the existing code.

- [ ] **Step 6: Wire the `List` branch to also use the extracted helpers**

Once `render_video_meta_row` and `apply_video_card_actions` exist, refactor the `ViewMode::List` arm to call them too, so all three modes share one code path for metadata + actions. This avoids the three-way drift the spec warned against.

- [ ] **Step 7: Build**

Run: `cargo build --release`
Expected: builds clean. Borrow-checker may complain about `self.texture(ctx, ...)` inside the loop while `&cards` is borrowed — the existing code already solves this with the deferred-flags pattern; mirror it.

- [ ] **Step 8: Smoke-test all three modes**

Run: `./target/release/catacomb`
For each of List/Card/Grid: click a video, double-click to play, toggle watched, toggle a flag, bulk-select. Confirm all actions work in all three modes.

- [ ] **Step 9: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): List/Card/Grid video list render modes"
```

---

## Task 10: Toolbar view-mode toggle + per-view override write

**Files:**
- Modify: `src/app.rs:4163-4166` (just above the `if cards.is_empty()` check, in the video list header area)

- [ ] **Step 1: Add the 3-segment toggle**

Just before `ui.separator();` at line ~4164, insert:

```rust
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("View:").weak().small());
                            let current = self.view_mode_for(&self.sidebar_view.clone());
                            for (mode, label) in [
                                (ViewMode::List, "☰ List"),
                                (ViewMode::Card, "▢ Card"),
                                (ViewMode::Grid, "⊫ Grid"),
                            ] {
                                if ui.selectable_label(current == mode, label).clicked() {
                                    self.view_mode_overrides
                                        .insert(self.sidebar_view.clone(), mode);
                                }
                            }
                        });
```

- [ ] **Step 2: Build + smoke-test**

Run: `cargo build --release && ./target/release/catacomb`
Switch to "All Videos", toggle to Grid. Switch to a channel — confirm it still shows the default (override is per-view). Toggle a channel to Card, switch away and back — confirm Card persists.

- [ ] **Step 3: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): view-mode toolbar toggle with per-view override"
```

---

## Task 11: Settings UI for default view mode

**Files:**
- Modify: `src/app.rs:3182` (the Settings screen, after the theme picker row)

- [ ] **Step 1: Add a default-view-mode combo row**

After the Theme combo row ends at `src/app.rs:3182` (`ui.end_row();`), insert a new row in the same `egui::Grid`:

```rust
                        ui.label("Default view:");
                        egui::ComboBox::from_id_salt("default_view_mode_combo")
                            .selected_text(match self.default_view_mode {
                                ViewMode::List => "List",
                                ViewMode::Card => "Card",
                                ViewMode::Grid => "Grid",
                            })
                            .show_ui(ui, |ui| {
                                for (mode, label) in [
                                    (ViewMode::List, "List"),
                                    (ViewMode::Card, "Card"),
                                    (ViewMode::Grid, "Grid"),
                                ] {
                                    if ui
                                        .selectable_label(self.default_view_mode == mode, label)
                                        .clicked()
                                    {
                                        self.default_view_mode = mode;
                                        self.config.ui.default_view_mode = match mode {
                                            ViewMode::List => "list",
                                            ViewMode::Card => "card",
                                            ViewMode::Grid => "grid",
                                        }.to_string();
                                    }
                                }
                            });
                        ui.end_row();
```

- [ ] **Step 2: Build + smoke-test**

Run: `cargo build --release && ./target/release/catacomb`
Open Settings, change Default view to Grid, restart the app. Confirm views with no override now default to Grid.

- [ ] **Step 3: Verify config persists**

After changing default view and quitting, check the written `config.toml` has `default_view_mode = "grid"` under `[ui]`. (Don't commit `config.toml`.)

- [ ] **Step 4: Commit**

```bash
git add src/app.rs
git commit -m "feat(app): Settings picker for default view mode"
```

---

## Task 12: Typography & spacing refresh

**Files:**
- Modify: `src/app.rs` — base font sizes via `ctx.set_style` / `FontDefinitions`, and per-call text sizes in the card renderers

- [ ] **Step 1: Bump base body text and set style spacing at construction**

At construction (near `src/app.rs:371`, after `theme::apply`), add:

```rust
        // Base text scale: bump body from ~12px to ~13px for legibility.
        let mut style = egui::Style::default();
        style.spacing.item_spacing = egui::vec2(8.0, 5.0);
        style.spacing.button_padding = egui::vec2(6.0, 3.0);
        cc.egui_ctx.set_style(style);
```

- [ ] **Step 2: Adjust card text sizes in `render_video_meta_row`**

In the extracted helper (Task 9), the title uses `egui::RichText::new(&card.title).strong()` — bump its size to 14.0 (proportional). The metadata line uses `.small()`; leave that, but ensure consistent separator spacing (replace `·` ad-hoc joins with ` · ` between every metadata item).

- [ ] **Step 3: Build + visual smoke-test**

Run: `cargo build --release && ./target/release/catacomb`
Confirm text is slightly larger, spacing more breathable, metadata line consistent across modes.

- [ ] **Step 4: Commit**

```bash
git add src/app.rs
git commit -m "style(app): bump base font + spacing, tidy metadata rhythm"
```

---

## Task 13: Full verification + docs sync

**Files:**
- Run tests, run app, verify spec coverage.

- [ ] **Step 1: Run the full test suite**

Run: `cargo test --release`
Expected: all pass (existing tests + new theme/config tests).

- [ ] **Step 2: Manual theme tour**

Run: `./target/release/catacomb`
Switch through all 19 themes. For 2–3 themes, exercise List/Card/Grid + select/play/bulk/watched/flag. Confirm accent rings track the theme in every mode.

- [ ] **Step 3: Verify the public doc is still accurate**

Read `docs/src/theming.md` (created earlier). Confirm the theme table count (19) and the view-mode description match what shipped. Fix any drift inline.

- [ ] **Step 4: Final commit (docs sync if changed)**

```bash
git add docs/src/theming.md  # only if changed
git commit -m "docs(theming): sync with shipped 19 themes + view modes"
```

---

## Self-review notes

- **Spec coverage:** §1 themes → Tasks 1,2,3. §2 accents → Tasks 1,5,6. §3 view toggle → Tasks 4,7,9,10,11. §4 default polish → Task 2. §5 typography → Task 12. §6 placeholders → Task 8. All spec sections covered.
- **Hex literal values** in Task 3 have been checked and corrected; build should be clean on first try.
- **Borrow-checker risk** in Task 9 is flagged; the existing deferred-flags pattern is the mitigation.
- **`render_video_meta_row` / `apply_video_card_actions` bodies are intentionally not pasted** because they must be copied from the *current* code, which may have drifted since this plan was written. The signatures are the contract.
