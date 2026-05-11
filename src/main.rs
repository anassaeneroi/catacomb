#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod database;
mod downloader;
mod library;
mod tray;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([800.0, 500.0])
            .with_title("YouTube Backup"),
        ..Default::default()
    };
    eframe::run_native(
        "youtube-backup",
        native_options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
