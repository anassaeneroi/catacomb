use eframe::egui::{self, Color32, Stroke};

pub const THEMES: &[(&str, &str)] = &[
    ("dark", "Dark"),
    ("light", "Light"),
    ("dracula", "Dracula"),
    ("trans", "Trans"),
    ("emo-nocturnal", "Emo: Nocturnal"),
    ("emo-coffin", "Emo: Coffin"),
    ("emo-scene-queen", "Emo: Scene Queen"),
];

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

/// Look up the semantic accents for a theme name. Falls back to the dark
/// theme's accents for unknown names.
pub fn accents_for(name: &str) -> ThemeAccents {
    match name {
        "dark" => ThemeAccents { accent: hex(0x7aa2f7), success: hex(0x9ece6a), warning: hex(0xbb9af7) },
        "light" => ThemeAccents { accent: hex(0x2a5db0), success: hex(0x2e7d32), warning: hex(0x8e44ad) },
        "dracula" => ThemeAccents { accent: hex(0xbd93f9), success: hex(0x50fa7b), warning: hex(0xff79c6) },
        "trans" => ThemeAccents { accent: hex(0x55cdfc), success: hex(0x2e7d32), warning: hex(0xf7a8b8) },
        "emo-nocturnal" => ThemeAccents { accent: hex(0xff0090), success: hex(0x39ff14), warning: hex(0x00f5ff) },
        "emo-coffin" => ThemeAccents { accent: hex(0x8b0000), success: hex(0x39ff14), warning: hex(0xcc2222) },
        "emo-scene-queen" => ThemeAccents { accent: hex(0x39ff14), success: hex(0xff00ff), warning: hex(0x00f5ff) },
        // New themes (added in a later task — values match their palettes).
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
        _ => ThemeAccents { accent: hex(0x7aa2f7), success: hex(0x9ece6a), warning: hex(0xbb9af7) },
    }
}

pub fn apply(ctx: &egui::Context, name: &str) {
    let visuals = match name {
        "light" => egui::Visuals::light(),
        "dracula" => dracula(),
        "trans" => trans(),
        "emo-nocturnal" => emo_nocturnal(),
        "emo-coffin" => emo_coffin(),
        "emo-scene-queen" => emo_scene_queen(),
        _ => egui::Visuals::dark(),
    };
    ctx.set_visuals(visuals);
}

fn hex(v: u32) -> Color32 {
    Color32::from_rgb(((v >> 16) & 0xff) as u8, ((v >> 8) & 0xff) as u8, (v & 0xff) as u8)
}

fn dracula() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = hex(0x282a36);
    v.window_fill = hex(0x282a36);
    v.extreme_bg_color = hex(0x1e2029);
    v.faint_bg_color = hex(0x343746);
    v.code_bg_color = hex(0x1e2029);
    v.selection.bg_fill = hex(0x44475a);
    v.selection.stroke = Stroke::new(1.0, hex(0xbd93f9));
    v.hyperlink_color = hex(0x8be9fd);
    // Don't override text globally — let each widget's fg_stroke pick the
    // contrast-appropriate colour for its state. With an override the
    // "active" purple button (light bg) would render light-on-light text.
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = hex(0x343746);
    v.widgets.noninteractive.weak_bg_fill = hex(0x2f3242);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0xf8f8f2));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0x44475a));
    v.widgets.inactive.bg_fill = hex(0x44475a);
    v.widgets.inactive.weak_bg_fill = hex(0x3d4059);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0xf8f8f2));
    v.widgets.hovered.bg_fill = hex(0x6272a4);
    v.widgets.hovered.weak_bg_fill = hex(0x5566a0);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0xf8f8f2));
    v.widgets.active.bg_fill = hex(0xbd93f9);
    v.widgets.active.weak_bg_fill = hex(0xaa80f0);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0x282a36));
    v.widgets.open.bg_fill = hex(0x6272a4);
    v.window_stroke = Stroke::new(1.0, hex(0x6272a4));
    v
}

// Trans flag: light blue #55cdfc, pink #f7a8b8, white #ffffff
fn trans() -> egui::Visuals {
    let mut v = egui::Visuals::light();
    v.panel_fill = hex(0xe8f7fd);
    v.window_fill = hex(0xfef0f4);
    v.extreme_bg_color = hex(0xffffff);
    v.faint_bg_color = hex(0xf5fbfe);
    v.code_bg_color = hex(0xf0f9fe);
    v.selection.bg_fill = hex(0x55cdfc);
    v.selection.stroke = Stroke::new(1.0, hex(0x2288cc));
    v.hyperlink_color = hex(0x0055aa);
    // Previously this forced hot-pink text everywhere; that wrecked
    // contrast on the pink inactive button bg. Per-widget fg_stroke now
    // handles colour per state.
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = hex(0xfce8f2);
    v.widgets.noninteractive.weak_bg_fill = hex(0xfdf4f8);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0x2a1430));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0xf7a8b8));
    v.widgets.inactive.bg_fill = hex(0xf7a8b8);
    v.widgets.inactive.weak_bg_fill = hex(0xfcccd8);
    // Pure black against pink so button labels stay legible.
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0x000000));
    v.widgets.hovered.bg_fill = hex(0x55cdfc);
    v.widgets.hovered.weak_bg_fill = hex(0x88ddfd);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0x111111));
    v.widgets.active.bg_fill = hex(0x2288cc);
    v.widgets.active.weak_bg_fill = hex(0x44aaee);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0xffffff));
    v.widgets.open.bg_fill = hex(0xf7a8b8);
    v.window_stroke = Stroke::new(1.0, hex(0xf7a8b8));
    v
}

// Neon pink on black — 2000s club/Hot Topic energy
fn emo_nocturnal() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = hex(0x0a0a0a);
    v.window_fill = hex(0x0d0d0d);
    v.extreme_bg_color = hex(0x000000);
    v.faint_bg_color = hex(0x111111);
    v.code_bg_color = hex(0x050505);
    v.selection.bg_fill = hex(0xff0090);
    v.selection.stroke = Stroke::new(1.0, hex(0xff66c0));
    v.hyperlink_color = hex(0x00f5ff);
    // Without an override the "active" hot-pink button gets its intended
    // pure-white text instead of a washed-out light grey on hot pink.
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = hex(0x1a1a1a);
    v.widgets.noninteractive.weak_bg_fill = hex(0x141414);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0xe0e0e0));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0x2a2a2a));
    v.widgets.inactive.bg_fill = hex(0x1f0018);
    v.widgets.inactive.weak_bg_fill = hex(0x180013);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0xd0d0d0));
    v.widgets.hovered.bg_fill = hex(0x8b004d);
    v.widgets.hovered.weak_bg_fill = hex(0x660038);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0xff88cc));
    v.widgets.active.bg_fill = hex(0xff0090);
    v.widgets.active.weak_bg_fill = hex(0xcc0070);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0xffffff));
    v.widgets.open.bg_fill = hex(0x8b004d);
    v.window_stroke = Stroke::new(1.0, hex(0xff0090));
    v.warn_fg_color = hex(0xffcc00);
    v.error_fg_color = hex(0xff0090);
    v
}

// Blood red on deep purple-black — cemetery goth
fn emo_coffin() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = hex(0x0d0009);
    v.window_fill = hex(0x110010);
    v.extreme_bg_color = hex(0x06000a);
    v.faint_bg_color = hex(0x150014);
    v.code_bg_color = hex(0x080008);
    v.selection.bg_fill = hex(0x8b0000);
    v.selection.stroke = Stroke::new(1.0, hex(0xcc2222));
    v.hyperlink_color = hex(0xcc2222);
    // Per-widget fg_stroke handles colour — the explicit values below already
    // contrast properly against each state's bg_fill.
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = hex(0x1a0018);
    v.widgets.noninteractive.weak_bg_fill = hex(0x140012);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0xb0b0b0));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0x3a0030));
    v.widgets.inactive.bg_fill = hex(0x230020);
    v.widgets.inactive.weak_bg_fill = hex(0x1c0018);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0xc0c0c0));
    v.widgets.hovered.bg_fill = hex(0x5a0010);
    v.widgets.hovered.weak_bg_fill = hex(0x440008);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0xdd8888));
    v.widgets.active.bg_fill = hex(0x8b0000);
    v.widgets.active.weak_bg_fill = hex(0x700000);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0xe0e0e0));
    v.widgets.open.bg_fill = hex(0x5a0010);
    v.window_stroke = Stroke::new(1.0, hex(0x8b0000));
    v.warn_fg_color = hex(0xffaa00);
    v.error_fg_color = hex(0xff3333);
    v
}

// Neon lime and magenta on dark navy — MySpace/scene queen era
fn emo_scene_queen() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = hex(0x080818);
    v.window_fill = hex(0x0a0a1e);
    v.extreme_bg_color = hex(0x04040e);
    v.faint_bg_color = hex(0x0d0d22);
    v.code_bg_color = hex(0x060613);
    v.selection.bg_fill = hex(0x39ff14);
    v.selection.stroke = Stroke::new(1.0, hex(0x66ff44));
    v.hyperlink_color = hex(0xff00ff);
    v.override_text_color = None; // let widget fg_stroke handle per-state text color
    v.widgets.noninteractive.bg_fill = hex(0x111128);
    v.widgets.noninteractive.weak_bg_fill = hex(0x0d0d20);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0xc8c8ff));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0x222244));
    v.widgets.inactive.bg_fill = hex(0x0d1a0a);
    v.widgets.inactive.weak_bg_fill = hex(0x0a1408);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0xc0c0f0));
    v.widgets.hovered.bg_fill = hex(0x1a3d12);
    v.widgets.hovered.weak_bg_fill = hex(0x14300e);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0x88ff66));
    v.widgets.active.bg_fill = hex(0x39ff14);
    v.widgets.active.weak_bg_fill = hex(0x2acc10);
    v.widgets.active.fg_stroke = Stroke::new(2.0, hex(0x080818));
    v.widgets.open.bg_fill = hex(0x1a3d12);
    v.window_stroke = Stroke::new(1.0, hex(0x39ff14));
    v.warn_fg_color = hex(0xffcc00);
    v.error_fg_color = hex(0xff4444);
    v
}

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
        let a = accents_for("this-theme-does-not-exist");
        let dark = accents_for("dark");
        assert_eq!(a.accent, dark.accent);
    }

    #[test]
    fn accents_differ_across_themes() {
        let dark = accents_for("dark");
        let light = accents_for("light");
        assert_ne!(dark.accent, light.accent);
    }
}
