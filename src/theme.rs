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
    v.override_text_color = Some(hex(0xf8f8f2));
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
    v.override_text_color = Some(hex(0xcc0066)); // hot pink text throughout
    v.widgets.noninteractive.bg_fill = hex(0xfce8f2);
    v.widgets.noninteractive.weak_bg_fill = hex(0xfdf4f8);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0x444444));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0xf7a8b8));
    v.widgets.inactive.bg_fill = hex(0xf7a8b8);
    v.widgets.inactive.weak_bg_fill = hex(0xfcccd8);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0x333333));
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
    v.override_text_color = Some(hex(0xe8e8e8));
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
    v.override_text_color = Some(hex(0xc0c0c0));
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
