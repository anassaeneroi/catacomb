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
mod crash;
mod database;
mod disk_space;
mod download_options;
mod downloader;
mod error_class;
mod library;
mod maintenance;
mod platform;
mod plex;
mod stats;
mod theme;
mod tray;
mod web;
mod ytdlp_bin;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Load the config first — both modes share the channels_root path,
    // and the panic hook needs it before anything else runs so a crash
    // in the very early startup (DB open, tray init, ksni) still lands
    // in the crash log.
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let cfg_path = cwd.join("config.toml");
    let mut cfg = config::Config::load(&cfg_path)
        .unwrap_or_else(|_| config::Config::default_with_dir(cwd.join("channels")));

    // Install the persistent panic logger. Logs go alongside the SQLite
    // database; the parent of channels_root is the same "library root"
    // the desktop UI shows.
    let crash_dir = cfg.backup.directory.parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| cfg.backup.directory.clone());
    crash::install(&crash_dir);

    // --web [port] → run the web interface instead of the GUI
    if let Some(pos) = args.iter().position(|a| a == "--web") {
        // Override port if provided after --web
        if let Some(port_str) = args.get(pos + 1) {
            if let Ok(port) = port_str.parse::<u16>() {
                cfg.web.port = port;
            }
        }
        web::run(cfg);
    }

    // Start the optional system tray. Returns None if the icon decode
    // fails (shouldn't happen — it's embedded) or the thread won't spawn.
    // A Some result does NOT guarantee a visible tray icon — that depends
    // on whether a StatusNotifier host is registered (KDE: yes; GNOME:
    // requires AppIndicator extension; others: varies). When no host
    // exists, ksni quietly does nothing and the app behaves as if the
    // tray flag were off.
    const TRAY_ICON_PNG: &[u8] = include_bytes!("../icon.png");
    let tray = tray::start(TRAY_ICON_PNG);

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
        Box::new(|cc| Ok(Box::new(app::App::new(cc, tray)))),
    )
}
