use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use eframe::egui;

use crate::config::Config;
use crate::downloader::{detect_url_kind, Downloader, JobState, UrlKind};
use crate::library::{self, Video};
use crate::theme;

struct Card {
    channel_name: String,
    title: String,
    id: String,
    video_path: Option<PathBuf>,
    thumb_path: Option<PathBuf>,
    has_live_chat: bool,
}

pub struct App {
    config: Config,
    config_path: PathBuf,
    channels_root: PathBuf,
    library: Vec<library::Channel>,
    selected_channel: Option<usize>,
    selected_playlist: Option<(usize, usize)>,
    selected_video: Option<String>,
    search: String,
    downloader: Downloader,
    show_downloads: bool,
    show_settings: bool,
    dl_url: String,
    textures: HashMap<PathBuf, Option<egui::TextureHandle>>,
    decode_budget: u32,
    desc_cache: HashMap<PathBuf, String>,
    status: String,
    settings_dir: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let config_path = cwd.join("config.toml");

        let config = match Config::load(&config_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Warning: failed to load config.toml: {e}. Using defaults.");
                Config::default_with_dir(cwd.join("channels"))
            }
        };

        theme::apply(&cc.egui_ctx, &config.ui.theme);

        let channels_root = config.backup.directory.clone();
        let settings_dir = channels_root.display().to_string();
        let _ = std::fs::create_dir_all(&channels_root);
        let library = library::scan_channels(&channels_root);
        let status = format!(
            "{} channels, {} videos",
            library.len(),
            library.iter().map(|c| c.total_videos()).sum::<usize>()
        );

        Self {
            config,
            config_path,
            channels_root: channels_root.clone(),
            library,
            selected_channel: None,
            selected_playlist: None,
            selected_video: None,
            search: String::new(),
            downloader: Downloader::new(channels_root),
            show_downloads: false,
            show_settings: false,
            dl_url: String::new(),
            textures: HashMap::new(),
            decode_budget: 0,
            desc_cache: HashMap::new(),
            status,
            settings_dir,
        }
    }

    fn rescan(&mut self) {
        self.library = library::scan_channels(&self.channels_root);
        self.selected_channel = None;
        self.selected_playlist = None;
        self.selected_video = None;
        self.desc_cache.clear();
        self.textures.clear();
        self.status = format!(
            "Rescanned: {} channels, {} videos",
            self.library.len(),
            self.library.iter().map(|c| c.total_videos()).sum::<usize>()
        );
    }

    fn cards(&self) -> Vec<Card> {
        let query = self.search.trim().to_lowercase();

        let channel_indices: Vec<usize> = if let Some(ci) = self.selected_channel {
            vec![ci]
        } else {
            (0..self.library.len()).collect()
        };

        let mut cards = Vec::new();
        for ci in channel_indices {
            let Some(channel) = self.library.get(ci) else { continue };

            let playlist_filter = match self.selected_playlist {
                Some((pci, pi)) if pci == ci => Some(pi),
                _ => None,
            };

            if let Some(pi) = playlist_filter {
                let Some(playlist) = channel.playlists.get(pi) else { continue };
                for v in &playlist.videos {
                    if !query.is_empty()
                        && !v.title.to_lowercase().contains(&query)
                        && !v.id.to_lowercase().contains(&query)
                    {
                        continue;
                    }
                    cards.push(Card {
                        channel_name: channel.name.clone(),
                        title: v.title.clone(),
                        id: v.id.clone(),
                        video_path: v.video_path.clone(),
                        thumb_path: v.thumb_path.clone(),
                        has_live_chat: v.has_live_chat,
                    });
                }
            } else {
                let all_videos: Vec<&library::Video> = channel
                    .videos
                    .iter()
                    .chain(channel.playlists.iter().flat_map(|p| p.videos.iter()))
                    .collect();
                for v in all_videos {
                    if !query.is_empty()
                        && !v.title.to_lowercase().contains(&query)
                        && !v.id.to_lowercase().contains(&query)
                    {
                        continue;
                    }
                    cards.push(Card {
                        channel_name: channel.name.clone(),
                        title: v.title.clone(),
                        id: v.id.clone(),
                        video_path: v.video_path.clone(),
                        thumb_path: v.thumb_path.clone(),
                        has_live_chat: v.has_live_chat,
                    });
                }
            }
        }
        cards
    }

    fn find_video_by_id(&self, id: &str) -> Option<(Video, String)> {
        for channel in &self.library {
            if let Some(v) = channel.videos.iter().find(|v| v.id == id) {
                return Some((v.clone(), channel.name.clone()));
            }
            for playlist in &channel.playlists {
                if let Some(v) = playlist.videos.iter().find(|v| v.id == id) {
                    return Some((v.clone(), channel.name.clone()));
                }
            }
        }
        None
    }

    fn texture(&mut self, ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
        if let Some(slot) = self.textures.get(path) {
            return slot.clone();
        }
        if self.decode_budget == 0 {
            ctx.request_repaint();
            return None;
        }
        self.decode_budget -= 1;
        let handle = decode_thumbnail(ctx, path);
        self.textures.insert(path.to_path_buf(), handle.clone());
        handle
    }

    fn description(&mut self, video: &Video) -> String {
        let Some(path) = &video.description_path else {
            return "(no description file)".to_string();
        };
        if let Some(text) = self.desc_cache.get(path) {
            return text.clone();
        }
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| format!("(could not read description: {e})"));
        self.desc_cache.insert(path.clone(), text.clone());
        text
    }

    fn play(&mut self, path: &Path) {
        let cmd = self.config.player.command.clone();
        match Command::new(&cmd).arg(path).spawn() {
            Ok(_) => self.status = format!("Playing {}", file_label(path)),
            Err(_) => match Command::new("xdg-open").arg(path).spawn() {
                Ok(_) => self.status = format!("Opened {} in default player", file_label(path)),
                Err(e) => self.status = format!("Couldn't open {}: {e}", file_label(path)),
            },
        }
    }

    fn open_in_file_manager(&mut self, path: &Path) {
        let target = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
        if let Err(e) = Command::new("xdg-open").arg(target).spawn() {
            self.status = format!("Couldn't open folder: {e}");
        }
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.heading("yt-offline");
                ui.separator();
                ui.label("🔍");
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("filter by title or id")
                        .desired_width(260.0),
                );
                if !self.search.is_empty() && ui.button("✖").on_hover_text("clear").clicked() {
                    self.search.clear();
                }
                ui.separator();
                if ui.button("⟳ Rescan").clicked() {
                    self.rescan();
                }
                let dl_label = if self.show_downloads { "⬇ Downloads ▸" } else { "⬇ Downloads" };
                if ui.selectable_label(self.show_downloads, dl_label).clicked() {
                    self.show_downloads = !self.show_downloads;
                }
                if ui.selectable_label(self.show_settings, "⚙ Settings").clicked() {
                    self.show_settings = !self.show_settings;
                    if self.show_settings {
                        self.settings_dir = self.channels_root.display().to_string();
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(&self.status).weak());
                });
            });
            ui.add_space(2.0);
        });
    }

    fn channel_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("channels")
            .resizable(true)
            .default_width(230.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.heading("Channels");
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let total: usize = self.library.iter().map(|c| c.total_videos()).sum();
                    if ui
                        .selectable_label(
                            self.selected_channel.is_none(),
                            format!("⊞ All  ({total})"),
                        )
                        .clicked()
                    {
                        self.selected_channel = None;
                        self.selected_playlist = None;
                        self.selected_video = None;
                    }
                    ui.separator();
                    for i in 0..self.library.len() {
                        let is_selected = self.selected_channel == Some(i);
                        let (name, total, has_playlists) = {
                            let ch = &self.library[i];
                            (ch.name.clone(), ch.total_videos(), !ch.playlists.is_empty())
                        };
                        let label = format!("{}  ({})", name, total);
                        if ui
                            .selectable_label(is_selected && self.selected_playlist.is_none(), label)
                            .on_hover_text(self.library[i].path.display().to_string())
                            .clicked()
                        {
                            self.selected_channel = Some(i);
                            self.selected_playlist = None;
                            self.selected_video = None;
                        }
                        if is_selected && has_playlists {
                            let playlist_count = self.library[i].playlists.len();
                            for pi in 0..playlist_count {
                                let (pl_name, pl_len) = {
                                    let pl = &self.library[i].playlists[pi];
                                    (pl.name.clone(), pl.videos.len())
                                };
                                let is_pl = self.selected_playlist == Some((i, pi));
                                let pl_label =
                                    format!("    └ {}  ({})", pl_name, pl_len);
                                if ui.selectable_label(is_pl, pl_label).clicked() {
                                    self.selected_playlist = Some((i, pi));
                                    self.selected_video = None;
                                }
                            }
                        }
                    }
                });
            });
    }

    fn downloads_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("downloads")
            .resizable(true)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.heading("New download");
                ui.label("Video, playlist, or channel URL:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.dl_url)
                        .hint_text("https://www.youtube.com/…")
                        .desired_width(f32::INFINITY),
                );

                let kind = detect_url_kind(self.dl_url.trim());
                let (type_label, dest_preview) = match &kind {
                    UrlKind::Channel { handle } => {
                        ("Channel", format!("→ channels/{}/", handle))
                    }
                    UrlKind::Playlist => ("Playlist", "→ channels/<channel>/<playlist>/".to_string()),
                    UrlKind::Video => ("Video", "→ channels/<channel>/".to_string()),
                    UrlKind::Unknown => ("—", String::new()),
                };

                if !self.dl_url.trim().is_empty() {
                    ui.horizontal(|ui| {
                        ui.label("Type:");
                        ui.strong(type_label);
                    });
                    if !dest_preview.is_empty() {
                        ui.label(egui::RichText::new(&dest_preview).small().weak());
                    }
                }

                let ready = !self.dl_url.trim().is_empty();
                if ui.add_enabled(ready, egui::Button::new("⬇  Start download")).clicked() {
                    let url = self.dl_url.trim().to_string();
                    let dest = dest_preview.clone();
                    self.downloader.start(url, &kind);
                    self.status = format!("Downloading: {dest}");
                }

                ui.separator();
                ui.horizontal(|ui| {
                    ui.heading("Jobs");
                    if !self.downloader.jobs.is_empty()
                        && !self.downloader.any_running()
                        && ui.button("Clear finished").clicked()
                    {
                        self.downloader.jobs.retain(|j| j.state == JobState::Running);
                    }
                });
                if self.downloader.jobs.is_empty() {
                    ui.label(egui::RichText::new("Nothing queued yet.").weak());
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for job in self.downloader.jobs.iter().rev() {
                        let (text, color) = match job.state {
                            JobState::Running => ("running", egui::Color32::from_rgb(230, 200, 60)),
                            JobState::Done => ("done", egui::Color32::from_rgb(110, 200, 110)),
                            JobState::Failed => ("failed", egui::Color32::from_rgb(220, 110, 110)),
                        };
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.colored_label(color, text);
                                ui.label(&job.label);
                            });
                            ui.label(egui::RichText::new(&job.url).small().weak());
                            if job.state == JobState::Running {
                                ui.add(egui::ProgressBar::new(job.progress).show_percentage());
                            }
                            let last = job.log.last().map(String::as_str).unwrap_or("");
                            if !last.is_empty() {
                                ui.label(egui::RichText::new(last).small().monospace());
                            }
                            ui.collapsing("output log", |ui| {
                                egui::ScrollArea::vertical()
                                    .max_height(180.0)
                                    .auto_shrink([false, true])
                                    .stick_to_bottom(true)
                                    .show(ui, |ui| {
                                        for line in &job.log {
                                            ui.label(egui::RichText::new(line).small().monospace());
                                        }
                                    });
                            });
                        });
                    }
                });
            });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = self.show_settings;
        egui::Window::new("⚙ Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(440.0)
            .show(ctx, |ui| {
                egui::Grid::new("settings_grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("Backup directory:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.settings_dir)
                                .desired_width(280.0)
                                .hint_text("/path/to/channels"),
                        );
                        ui.end_row();

                        ui.label("Player command:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.player.command)
                                .desired_width(280.0)
                                .hint_text("mpv"),
                        );
                        ui.end_row();

                        ui.label("Theme:");
                        egui::ComboBox::from_id_salt("theme_combo")
                            .selected_text(
                                theme::THEMES
                                    .iter()
                                    .find(|(id, _)| *id == self.config.ui.theme)
                                    .map(|(_, label)| *label)
                                    .unwrap_or("Dark"),
                            )
                            .show_ui(ui, |ui| {
                                for (id, label) in theme::THEMES {
                                    if ui
                                        .selectable_label(self.config.ui.theme == *id, *label)
                                        .clicked()
                                    {
                                        self.config.ui.theme = id.to_string();
                                        theme::apply(ctx, id);
                                    }
                                }
                            });
                        ui.end_row();
                    });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    if ui.button("Apply & Save").clicked() {
                        let new_dir = PathBuf::from(&self.settings_dir);
                        let dir_changed = new_dir != self.config.backup.directory;
                        self.config.backup.directory = new_dir.clone();
                        match self.config.save(&self.config_path) {
                            Ok(_) => self.status = "Settings saved.".to_string(),
                            Err(e) => self.status = format!("Error saving config: {e}"),
                        }
                        if dir_changed {
                            self.channels_root = new_dir.clone();
                            self.downloader.channels_root = new_dir;
                            let _ = std::fs::create_dir_all(&self.channels_root);
                            self.rescan();
                        }
                    }
                    ui.label(egui::RichText::new("Theme previews immediately; other changes apply on save.").weak().small());
                });
            });
        self.show_settings = open;
    }

    fn detail_panel(&mut self, ctx: &egui::Context) {
        let selected_id = match &self.selected_video {
            Some(id) => id.clone(),
            None => return,
        };
        let Some((video, _channel_name)) = self.find_video_by_id(&selected_id) else {
            self.selected_video = None;
            return;
        };
        egui::TopBottomPanel::bottom("detail")
            .resizable(true)
            .default_height(220.0)
            .min_height(80.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.heading(&video.title);
                    if video.video_path.is_some() && ui.button("▶ Play").clicked() {
                        if let Some(p) = video.video_path.clone() {
                            self.play(&p);
                        }
                    }
                    if let Some(p) = video.video_path.clone() {
                        if ui.button("📁 Show file").clicked() {
                            self.open_in_file_manager(&p);
                        }
                    }
                    if video.has_live_chat {
                        ui.label(egui::RichText::new("💬 has live chat").small().weak());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("id: {}", video.id)).monospace().weak(),
                        );
                        if ui.button("✖ close").clicked() {
                            self.selected_video = None;
                        }
                    });
                });
                ui.separator();
                let description = self.description(&video);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(description).wrap());
                    });
            });
    }

    fn video_list(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let cards = self.cards();
        let show_channel = self.selected_channel.is_none();
        ui.horizontal(|ui| {
            ui.label(format!("{} videos", cards.len()));
            if !self.search.trim().is_empty() {
                ui.label(
                    egui::RichText::new(format!("(filtered by \"{}\")", self.search.trim())).weak(),
                );
            }
        });
        ui.separator();
        if cards.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("Nothing here.").weak());
                ui.label(
                    egui::RichText::new(
                        "Drop a yt-dlp download into channels/<name>/, or use the Downloads panel.",
                    )
                    .small()
                    .weak(),
                );
            });
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let thumb_size = egui::vec2(176.0, 99.0);
            for card in &cards {
                let selected = self.selected_video.as_deref() == Some(card.id.as_str());
                let mut clicked_card = false;
                let mut play_card = false;
                ui.horizontal(|ui| {
                    let (rect, resp) =
                        ui.allocate_exact_size(thumb_size, egui::Sense::click());
                    let texture = card
                        .thumb_path
                        .as_ref()
                        .and_then(|p| self.texture(ctx, p));
                    match &texture {
                        Some(handle) => {
                            egui::Image::new(handle)
                                .maintain_aspect_ratio(true)
                                .paint_at(ui, rect);
                        }
                        None => {
                            ui.painter().rect_filled(
                                rect,
                                4.0,
                                egui::Color32::from_gray(38),
                            );
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "▶",
                                egui::FontId::proportional(26.0),
                                egui::Color32::from_gray(110),
                            );
                        }
                    }
                    if selected {
                        ui.painter().rect_stroke(
                            rect,
                            4.0,
                            egui::Stroke::new(2.0, egui::Color32::from_rgb(120, 170, 230)),
                        );
                    }
                    if resp.clicked() {
                        clicked_card = true;
                    }
                    if resp.double_clicked() {
                        play_card = true;
                    }

                    ui.vertical(|ui| {
                        if ui
                            .selectable_label(
                                selected,
                                egui::RichText::new(&card.title).strong(),
                            )
                            .clicked()
                        {
                            clicked_card = true;
                        }
                        ui.horizontal(|ui| {
                            if show_channel {
                                ui.label(
                                    egui::RichText::new(&card.channel_name).small().weak(),
                                );
                                ui.label(egui::RichText::new("·").weak());
                            }
                            ui.label(egui::RichText::new(&card.id).small().monospace().weak());
                            if card.has_live_chat {
                                ui.label(egui::RichText::new("· 💬").small().weak());
                            }
                            if card.video_path.is_none() {
                                ui.label(
                                    egui::RichText::new("· no video file")
                                        .small()
                                        .color(egui::Color32::from_rgb(200, 140, 90)),
                                );
                            }
                        });
                        ui.horizontal(|ui| {
                            if card.video_path.is_some() && ui.small_button("▶ Play").clicked() {
                                play_card = true;
                            }
                            if ui.small_button("Details").clicked() {
                                clicked_card = true;
                            }
                        });
                    });
                });
                if play_card {
                    if let Some(p) = card.video_path.clone() {
                        self.play(&p);
                    }
                    self.selected_video = Some(card.id.clone());
                } else if clicked_card {
                    self.selected_video = Some(card.id.clone());
                }
                ui.separator();
            }
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.downloader.poll();
        if self.downloader.any_running() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        self.decode_budget = 6;

        self.top_bar(ctx);
        self.channel_panel(ctx);
        if self.show_downloads {
            self.downloads_panel(ctx);
        }
        self.settings_window(ctx);
        self.detail_panel(ctx);
        egui::CentralPanel::default().show(ctx, |ui| {
            self.video_list(ctx, ui);
        });
    }
}

fn decode_thumbnail(ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
    let image = image::open(path).ok()?;
    let image = image.thumbnail(384, 216);
    let rgba = image.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    let color_image = egui::ColorImage::from_rgba_unmultiplied([w, h], rgba.as_raw());
    Some(ctx.load_texture(path.to_string_lossy(), color_image, egui::TextureOptions::LINEAR))
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}
