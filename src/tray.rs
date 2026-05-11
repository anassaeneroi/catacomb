use tray_icon::{TrayIconBuilder, menu::Menu};
use std::path::PathBuf;

pub fn create_tray_icon(icon_path: Option<PathBuf>) -> Result<tray_icon::TrayIcon, Box<dyn std::error::Error>> {
    let menu = Menu::new();

    let icon = if let Some(path) = icon_path {
        load_icon_from_file(&path)?
    } else {
        create_default_icon()?
    };

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("YouTube Backup\nDownload and manage YouTube channel backups")
        .with_icon(icon)
        .build()?;

    Ok(tray)
}

fn load_icon_from_file(path: &PathBuf) -> Result<tray_icon::Icon, Box<dyn std::error::Error>> {
    let img = image::open(path)?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Ok(tray_icon::Icon::from_rgba(rgba.into_raw(), w, h)?)
}

fn create_default_icon() -> Result<tray_icon::Icon, Box<dyn std::error::Error>> {
    let mut rgba = vec![0u8; 64 * 64 * 4];
    for chunk in rgba.chunks_mut(4) {
        chunk[0] = 255; // red
        chunk[3] = 255; // alpha
    }
    Ok(tray_icon::Icon::from_rgba(rgba, 64, 64)?)
}
