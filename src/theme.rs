use eframe::egui::{self, Color32, Stroke};

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

fn hex(v: u32) -> Color32 {
    Color32::from_rgb(((v >> 16) & 0xff) as u8, ((v >> 8) & 0xff) as u8, (v & 0xff) as u8)
}

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
