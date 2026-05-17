//! yt-offline — desktop and web app for archiving YouTube content with yt-dlp.
//!
//! # Usage
//!
//! * **GUI mode** (default): `yt-offline`
//! * **Web mode**: `yt-offline --web [PORT]` — starts a headless HTTP server
//!   on the configured port (default 8080).
//!
//! Configuration is read from `config.toml` in the current working directory.
//! See [`config`] for all available options.
//!
//! Licensed under the GNU Affero General Public License v3 or later (AGPL-3.0+).
//! Source code must be made available to network users per AGPL §13.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod database;
mod downloader;
mod library;
mod theme;
mod web;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // --web [port] → run the web interface instead of the GUI
    if let Some(pos) = args.iter().position(|a| a == "--web") {
        let mut cfg = {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            config::Config::load(&cwd.join("config.toml"))
                .unwrap_or_else(|_| config::Config::default_with_dir(cwd.join("channels")))
        };
        // Override port if provided after --web
        if let Some(port_str) = args.get(pos + 1) {
            if let Ok(port) = port_str.parse::<u16>() {
                cfg.web.port = port;
            }
        }
        web::run(cfg);
    }

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([800.0, 500.0])
            .with_title("yt-offline"),
        ..Default::default()
    };
    eframe::run_native(
        "yt-offline",
        native_options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
