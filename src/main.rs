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
mod autotag;
mod config;
mod crash;
mod database;
mod disk_space;
mod download_options;
mod downloader;
mod error_class;
mod feed;
mod fingerprint;
mod library;
mod maintenance;
mod platform;
mod plex;
mod pot_provider;
mod remote;
mod stats;
mod theme;
mod tray;
mod util;
mod vtt;
mod web;
mod ytdlp_bin;

fn renderer_from_name(requested: Option<&str>) -> eframe::Renderer {
    match requested.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("glow" | "opengl" | "gl") => eframe::Renderer::Glow,
        Some("wgpu" | "vulkan" | "vk") | None => eframe::Renderer::Wgpu,
        Some(name) => {
            eprintln!(
                "Warning: unknown renderer '{name}' (expected 'wgpu' or 'glow'); using wgpu."
            );
            eframe::Renderer::Wgpu
        }
    }
}

fn renderer_from_args(args: &[String]) -> eframe::Renderer {
    let requested = args
        .iter()
        .position(|a| a == "--renderer")
        .and_then(|pos| args.get(pos + 1).map(String::as_str));

    if requested.is_some() {
        renderer_from_name(requested)
    } else {
        let env_renderer = std::env::var("YT_OFFLINE_RENDERER").ok();
        renderer_from_name(env_renderer.as_deref())
    }
}

/// Reattach stdout/stderr to the parent console on Windows.
///
/// Release builds set `windows_subsystem = "windows"`, so the process starts
/// detached from any console — correct for the GUI (no flashing console
/// window), but it means a `--web` or other CLI invocation launched from a
/// terminal would print nothing. `AttachConsole(ATTACH_PARENT_PROCESS)` hooks
/// us back onto the launching terminal's console when there is one (e.g. run
/// from PowerShell/cmd). If the process has no parent console (double-clicked
/// from Explorer) the call fails harmlessly and we stay silent, which is the
/// desired GUI behavior. No-op on every non-Windows platform.
#[cfg(windows)]
fn attach_windows_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    // SAFETY: AttachConsole takes no pointers and is safe to call with an
    // invalid/absent parent console — it just returns 0. We don't touch the
    // returned handles; the CRT picks up the now-attached console on the next
    // stdout/stderr write.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_windows_console() {}

fn main() -> eframe::Result<()> {
    attach_windows_console();

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
        // Default to wgpu (Vulkan): the glow/OpenGL path crashes on some
        // NVIDIA + Wayland maximizes (Glutin EGL_BAD_ALLOC). Keep a launch
        // escape hatch for systems where Vulkan/wgpu opens a blank window:
        // `YT_OFFLINE_RENDERER=glow yt-offline` or `yt-offline --renderer glow`.
        renderer: renderer_from_args(&args),
        ..Default::default()
    };
    eframe::run_native(
        "yt-offline",
        native_options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc, tray)))),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn renderer_name_accepts_default_and_aliases() {
        assert_eq!(super::renderer_from_name(None), eframe::Renderer::Wgpu);
        assert_eq!(
            super::renderer_from_name(Some("wgpu")),
            eframe::Renderer::Wgpu
        );
        assert_eq!(
            super::renderer_from_name(Some("vulkan")),
            eframe::Renderer::Wgpu
        );
        assert_eq!(
            super::renderer_from_name(Some("glow")),
            eframe::Renderer::Glow
        );
        assert_eq!(
            super::renderer_from_name(Some("opengl")),
            eframe::Renderer::Glow
        );
        assert_eq!(
            super::renderer_from_name(Some("unknown")),
            eframe::Renderer::Wgpu
        );
    }

    #[test]
    fn renderer_arg_overrides_env_parser_path() {
        let args = vec![
            "yt-offline".to_string(),
            "--renderer".to_string(),
            "gl".to_string(),
        ];
        assert_eq!(super::renderer_from_args(&args), eframe::Renderer::Glow);
    }
}
