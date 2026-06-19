//! Linux system-tray integration via StatusNotifierItem (SNI) using `ksni`.
//!
//! Why ksni rather than the cross-platform `tray-icon` crate: ksni is pure
//! zbus (no GTK dep), which matches the rest of the stack (rfd uses
//! xdg-portal, notify-rust uses zbus). On KDE the tray "just works"; on
//! GNOME it requires the AppIndicator extension; on other DEs it varies.
//! When no SNI host is registered, ksni silently does nothing — the app
//! still runs, just without a tray icon. That's the right failure mode
//! for an optional convenience feature.
//!
//! # Threading
//!
//! Ksni needs a tokio runtime. The desktop app doesn't have one (only the
//! web server starts one on demand), so we spin a dedicated current-thread
//! runtime on a background OS thread purely for the tray. Events from the
//! tray menu come back to the egui app via a [`mpsc`] channel which the
//! main loop drains each frame.
//!
//! Egui itself is *not* told about the tray; only [`App`](crate::app::App)
//! polls the [`TrayHandle`] and dispatches `ViewportCommand`s. That keeps
//! the tray entirely optional — if `TrayController::start` fails the app
//! continues normally.

use std::sync::mpsc::Receiver;

/// Menu events emitted by the tray. The main app drains these in
/// [`eframe::App::update`] and translates them into viewport commands or
/// an exit flag.
#[derive(Clone, Copy, Debug)]
pub enum TrayEvent {
    /// Show + focus the main window. Bound to the left-click handler so a
    /// single click on the icon brings the app back from "hidden to tray".
    Show,
    /// Hide the main window. Useful for minimize-to-tray on close.
    Hide,
    /// Quit the application. Wired to the menu's "Exit" item; the main
    /// loop responds by sending `ViewportCommand::Close` to actually exit.
    Quit,
}

/// Owned handle returned by [`start`]. Dropping it does *not* tear down
/// the tray — it lives on the background runtime until the process exits.
/// We just hold the receiver end of the menu-event channel.
pub struct TrayHandle {
    pub events: Receiver<TrayEvent>,
}

/// Internal tray model implementing `ksni::Tray`. Each method is called
/// by ksni when the SNI host needs a property; menu activations call the
/// boxed closures we store in `MenuItem::activate`, which forward into
/// the channel back to the main thread.
#[cfg(target_os = "linux")]
struct TrayModel {
    tx: std::sync::mpsc::Sender<TrayEvent>,
    /// PNG bytes for the icon, decoded once at construction. ksni asks
    /// for raw ARGB so we keep both forms.
    icon_argb: Vec<u8>,
    icon_w: i32,
    icon_h: i32,
}

#[cfg(target_os = "linux")]
impl ksni::Tray for TrayModel {
    fn id(&self) -> String {
        "Catacomb".into()
    }
    fn title(&self) -> String {
        "Catacomb".into()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Catacomb".into(),
            description: "Self-hosted archive for YouTube + friends".into(),
            ..Default::default()
        }
    }
    /// Returning a non-empty icon_pixmap takes precedence over icon_name.
    /// We embed the icon so the tray works regardless of icon-theme setup.
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![ksni::Icon {
            width: self.icon_w,
            height: self.icon_h,
            data: self.icon_argb.clone(),
        }]
    }
    /// Activate fires on left-click of the tray icon. Surfacing the main
    /// window is the most common "what did the user mean" interpretation.
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(TrayEvent::Show);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Show Catacomb".into(),
                activate: Box::new(|m: &mut Self| {
                    let _ = m.tx.send(TrayEvent::Show);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Hide window".into(),
                activate: Box::new(|m: &mut Self| {
                    let _ = m.tx.send(TrayEvent::Hide);
                }),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|m: &mut Self| {
                    let _ = m.tx.send(TrayEvent::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Spawn the tray on a dedicated background thread + tokio runtime.
///
/// `icon_png_bytes` is decoded to ARGB once and embedded as the tray
/// pixmap. Pass the bundled `icon.png` from the binary.
///
/// Returns `None` if the runtime / channel can't be set up. A `Some(_)`
/// return does *not* guarantee the icon is actually visible — that depends
/// on whether a StatusNotifier host is registered on the user's desktop.
#[cfg(target_os = "linux")]
pub fn start(icon_png_bytes: &[u8]) -> Option<TrayHandle> {
    // Decode the PNG eagerly so we fail fast if the embedded asset is bad.
    let img = image::load_from_memory(icon_png_bytes).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    // ksni wants ARGB (alpha first), but image gives us RGBA. Swap.
    let mut argb = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        argb.push(px[3]);
        argb.push(px[0]);
        argb.push(px[1]);
        argb.push(px[2]);
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let model = TrayModel {
        tx,
        icon_argb: argb,
        icon_w: w as i32,
        icon_h: h as i32,
    };

    std::thread::Builder::new()
        .name("catacomb-tray".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(_) => return,
            };
            rt.block_on(async move {
                use ksni::TrayMethods;
                // spawn() registers the SNI service and returns a handle
                // we don't need to keep alive — the service runs as a
                // background task on this runtime until the process exits.
                if model.spawn().await.is_err() {
                    // No SNI host or DBus issue. Park the thread so the
                    // runtime stays alive; that's harmless.
                }
                std::future::pending::<()>().await;
            });
        })
        .ok()?;

    Some(TrayHandle { events: rx })
}

/// Non-Linux stub: there's no StatusNotifierItem tray backend on Windows or
/// macOS yet (ksni is Linux/SNI-only), so the tray is simply absent and the
/// app runs windowed-only. A real implementation would need a per-OS backend
/// (e.g. `tray-icon`) behind this same signature. Returning `None` makes
/// `App` behave exactly as it does on Linux when no SNI host is registered.
#[cfg(not(target_os = "linux"))]
pub fn start(_icon_png_bytes: &[u8]) -> Option<TrayHandle> {
    None
}
