use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use eframe::egui;

use crate::config::Config;
use crate::database::Database;
use crate::downloader::{detect_url_kind, Downloader, JobState, UrlKind};
use crate::library::{self, Video};
use crate::theme;

const BROWSERS: &[(&str, &str)] = &[
    ("firefox", "Firefox"),
    ("chromium", "Chromium"),
    ("chrome", "Chrome"),
    ("opera", "Opera"),
    ("brave", "Brave"),
    ("vivaldi", "Vivaldi"),
    ("none", "None (no cookies)"),
];

#[derive(Clone, PartialEq)]
enum SortMode {
    Title,
    DurationAsc,
    DurationDesc,
    SizeAsc,
    SizeDesc,
}

#[derive(Clone, PartialEq)]
enum SidebarView {
    All,
    Channel(usize),
    Playlist(usize, usize),
    ContinueWatching,
}

struct Card {
    channel_name: String,
    title: String,
    id: String,
    video_path: Option<PathBuf>,
    thumb_path: Option<PathBuf>,
    has_live_chat: bool,
    duration_secs: Option<f64>,
    file_size: Option<u64>,
    watched: bool,
    resume_pos: Option<f64>,
}

pub struct App {
    config: Config,
    config_path: PathBuf,
    channels_root: PathBuf,
    library: Vec<library::Channel>,
    sidebar_view: SidebarView,
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
    db: Database,
    card_density: f32,
    sort_mode: SortMode,
    watched: HashSet<String>,
    resume_positions: HashMap<String, f64>,
    prev_job_states: HashMap<usize, JobState>,
    currently_playing: Option<String>,
    mpv_rx: Option<Receiver<(String, f64)>>,
    // Bulk selection
    bulk_mode: bool,
    bulk_selected: HashSet<String>,
    // Scheduler
    last_scheduled_check: Option<Instant>,
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

        let db_path = channels_root.join("yt-offline.db");
        let db = Database::open(&db_path)
            .unwrap_or_else(|_| Database::open_in_memory().expect("in-memory db failed"));
        let watched = db.get_watched().unwrap_or_default();
        let resume_positions = db.get_positions().unwrap_or_default();

        let browser = config.player.browser.clone();

        Self {
            config,
            config_path,
            channels_root: channels_root.clone(),
            library,
            sidebar_view: SidebarView::All,
            selected_video: None,
            search: String::new(),
            downloader: Downloader::new(channels_root, browser),
            show_downloads: false,
            show_settings: false,
            dl_url: String::new(),
            textures: HashMap::new(),
            decode_budget: 0,
            desc_cache: HashMap::new(),
            status,
            settings_dir,
            db,
            card_density: 1.0,
            sort_mode: SortMode::Title,
            watched,
            resume_positions,
            prev_job_states: HashMap::new(),
            currently_playing: None,
            mpv_rx: None,
            bulk_mode: false,
            bulk_selected: HashSet::new(),
            last_scheduled_check: None,
        }
    }

    fn rescan(&mut self) {
        self.library = library::scan_channels(&self.channels_root);
        self.sidebar_view = SidebarView::All;
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

        let mut cards = Vec::new();

        let add_video = |cards: &mut Vec<Card>, ch_name: &str, v: &library::Video| {
            if !query.is_empty()
                && !v.title.to_lowercase().contains(&query)
                && !v.id.to_lowercase().contains(&query)
            {
                return;
            }
            let resume_pos = self.resume_positions.get(&v.id).copied();
            cards.push(Card {
                channel_name: ch_name.to_string(),
                title: v.title.clone(),
                id: v.id.clone(),
                video_path: v.video_path.clone(),
                thumb_path: v.thumb_path.clone(),
                has_live_chat: v.has_live_chat,
                duration_secs: v.duration_secs,
                file_size: v.file_size,
                watched: self.watched.contains(&v.id),
                resume_pos,
            });
        };

        match &self.sidebar_view {
            SidebarView::ContinueWatching => {
                for ch in &self.library {
                    for v in ch.videos.iter().chain(ch.playlists.iter().flat_map(|p| p.videos.iter())) {
                        if self.resume_positions.contains_key(&v.id) {
                            add_video(&mut cards, &ch.name, v);
                        }
                    }
                }
                cards.sort_by(|a, b| {
                    b.resume_pos.unwrap_or(0.0)
                        .partial_cmp(&a.resume_pos.unwrap_or(0.0))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                return cards;
            }
            SidebarView::All => {
                for ch in &self.library {
                    for v in ch.videos.iter().chain(ch.playlists.iter().flat_map(|p| p.videos.iter())) {
                        add_video(&mut cards, &ch.name, v);
                    }
                }
            }
            SidebarView::Channel(ci) => {
                if let Some(ch) = self.library.get(*ci) {
                    for v in ch.videos.iter().chain(ch.playlists.iter().flat_map(|p| p.videos.iter())) {
                        add_video(&mut cards, &ch.name, v);
                    }
                }
            }
            SidebarView::Playlist(ci, pi) => {
                if let Some(ch) = self.library.get(*ci) {
                    if let Some(pl) = ch.playlists.get(*pi) {
                        for v in &pl.videos {
                            add_video(&mut cards, &ch.name, v);
                        }
                    }
                }
            }
        }

        match self.sort_mode {
            SortMode::Title => cards.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase())),
            SortMode::DurationAsc => cards.sort_by(|a, b| {
                a.duration_secs.unwrap_or(0.0)
                    .partial_cmp(&b.duration_secs.unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortMode::DurationDesc => cards.sort_by(|a, b| {
                b.duration_secs.unwrap_or(0.0)
                    .partial_cmp(&a.duration_secs.unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortMode::SizeAsc => cards.sort_by_key(|c| c.file_size.unwrap_or(0)),
            SortMode::SizeDesc => {
                cards.sort_by_key(|c| c.file_size.unwrap_or(0));
                cards.reverse();
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

    fn play_with_tracking(&mut self, path: &Path, video_id: String) {
        let cmd = self.config.player.command.clone();
        let use_mpv_ipc = cmd.contains("mpv");

        #[cfg(unix)]
        let sock_path = format!("/tmp/yt-offline-{video_id}.sock");

        let mut child_cmd = Command::new(&cmd);

        #[cfg(unix)]
        if use_mpv_ipc {
            child_cmd.arg(format!("--input-ipc-server={sock_path}"));
        }

        if let Some(&pos) = self.resume_positions.get(&video_id) {
            if pos > 5.0 && use_mpv_ipc {
                child_cmd.arg(format!("--start={}", pos as u64));
            }
        }

        child_cmd.arg(path);

        match child_cmd.spawn() {
            Ok(_) => {
                self.status = format!("Playing {}", file_label(path));
                self.currently_playing = Some(video_id.clone());

                #[cfg(unix)]
                if use_mpv_ipc {
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.mpv_rx = Some(rx);
                    let id = video_id.clone();
                    std::thread::spawn(move || {
                        spawn_mpv_tracker(sock_path, id, tx);
                    });
                }
            }
            Err(_) => {
                #[cfg(target_os = "macos")]
                let fallback = "open";
                #[cfg(not(target_os = "macos"))]
                let fallback = "xdg-open";

                match Command::new(fallback).arg(path).spawn() {
                    Ok(_) => self.status = format!("Opened {} in default player", file_label(path)),
                    Err(e) => self.status = format!("Couldn't open {}: {e}", file_label(path)),
                }
            }
        }
    }

    fn open_in_file_manager(&mut self, path: &Path) {
        let target = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
        #[cfg(target_os = "macos")]
        let cmd = "open";
        #[cfg(not(target_os = "macos"))]
        let cmd = "xdg-open";
        if let Err(e) = Command::new(cmd).arg(target).spawn() {
            self.status = format!("Couldn't open folder: {e}");
        }
    }

    fn toggle_watched(&mut self, video_id: &str) {
        let now_watched = !self.watched.contains(video_id);
        if let Ok(()) = self.db.set_watched(video_id, now_watched) {
            if now_watched {
                self.watched.insert(video_id.to_string());
            } else {
                self.watched.remove(video_id);
            }
        }
    }

    fn bulk_mark_watched(&mut self, watched: bool) {
        let ids: Vec<String> = self.bulk_selected.iter().cloned().collect();
        for id in &ids {
            if let Ok(()) = self.db.set_watched(id, watched) {
                if watched {
                    self.watched.insert(id.clone());
                } else {
                    self.watched.remove(id);
                }
            }
        }
        self.bulk_selected.clear();
        self.status = format!(
            "{} {} as {}watched",
            ids.len(),
            if ids.len() == 1 { "video" } else { "videos" },
            if watched { "" } else { "un" }
        );
    }

    fn run_scheduled_check(&mut self) {
        let mut count = 0;
        let urls: Vec<String> = self.library.iter()
            .filter_map(|ch| ch.meta.as_ref()?.channel_url.clone())
            .collect();
        for url in urls {
            let kind = detect_url_kind(&url);
            self.downloader.start(url, &kind);
            count += 1;
        }
        self.status = format!("Scheduled check: started {} channel downloads", count);
    }

    fn check_notifications(&mut self) {
        let jobs = &self.downloader.jobs;
        let mut finished: Vec<(String, bool)> = Vec::new();

        for (i, job) in jobs.iter().enumerate() {
            let prev = self.prev_job_states.get(&i).copied();
            if prev == Some(JobState::Running) && job.state != JobState::Running {
                finished.push((job.label.clone(), job.state == JobState::Done));
            }
        }

        // Rebuild snapshot
        self.prev_job_states = jobs.iter().enumerate()
            .map(|(i, j)| (i, j.state))
            .collect();

        for (label, ok) in finished {
            let summary = if ok {
                format!("Download complete: {label}")
            } else {
                format!("Download failed: {label}")
            };
            let _ = notify_rust::Notification::new()
                .summary("yt-offline")
                .body(&summary)
                .timeout(notify_rust::Timeout::Milliseconds(4000))
                .show();
        }
    }

    fn channel_total_size(ch: &library::Channel) -> u64 {
        ch.videos.iter()
            .chain(ch.playlists.iter().flat_map(|p| p.videos.iter()))
            .filter_map(|v| v.file_size)
            .sum()
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
                        .desired_width(200.0),
                );
                if !self.search.is_empty() && ui.button("✖").on_hover_text("clear").clicked() {
                    self.search.clear();
                }
                ui.separator();
                ui.label("Size:");
                ui.add(
                    egui::Slider::new(&mut self.card_density, 0.5_f32..=2.0_f32)
                        .show_value(false)
                        .step_by(0.1),
                );
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
                    let resume_count = self.resume_positions.len();

                    if ui
                        .selectable_label(
                            self.sidebar_view == SidebarView::All,
                            format!("⊞ All  ({total})"),
                        )
                        .clicked()
                    {
                        self.sidebar_view = SidebarView::All;
                        self.selected_video = None;
                    }

                    if resume_count > 0 {
                        if ui
                            .selectable_label(
                                self.sidebar_view == SidebarView::ContinueWatching,
                                format!("▶ Continue Watching  ({resume_count})"),
                            )
                            .clicked()
                        {
                            self.sidebar_view = SidebarView::ContinueWatching;
                            self.selected_video = None;
                        }
                    }

                    ui.separator();

                    for i in 0..self.library.len() {
                        let (name, total, has_playlists, size_bytes) = {
                            let ch = &self.library[i];
                            (
                                ch.name.clone(),
                                ch.total_videos(),
                                !ch.playlists.is_empty(),
                                Self::channel_total_size(ch),
                            )
                        };

                        let is_ch_selected = matches!(self.sidebar_view, SidebarView::Channel(ci) if ci == i)
                            || matches!(self.sidebar_view, SidebarView::Playlist(ci, _) if ci == i);

                        let size_str = if size_bytes > 0 {
                            format!(" · {}", format_size(size_bytes))
                        } else {
                            String::new()
                        };
                        let label = format!("{}  ({}{})", name, total, size_str);

                        let ch_selected_no_pl = matches!(self.sidebar_view, SidebarView::Channel(ci) if ci == i);
                        if ui
                            .selectable_label(ch_selected_no_pl, label)
                            .on_hover_text(self.library[i].path.display().to_string())
                            .clicked()
                        {
                            self.sidebar_view = SidebarView::Channel(i);
                            self.selected_video = None;
                        }

                        if is_ch_selected && has_playlists {
                            let playlist_count = self.library[i].playlists.len();
                            for pi in 0..playlist_count {
                                let (pl_name, pl_len) = {
                                    let pl = &self.library[i].playlists[pi];
                                    (pl.name.clone(), pl.videos.len())
                                };
                                let is_pl = matches!(self.sidebar_view, SidebarView::Playlist(ci, pli) if ci == i && pli == pi);
                                let pl_label = format!("    └ {}  ({})", pl_name, pl_len);
                                if ui.selectable_label(is_pl, pl_label).clicked() {
                                    self.sidebar_view = SidebarView::Playlist(i, pi);
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
                        self.prev_job_states.clear();
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
            .default_width(480.0)
            .show(ctx, |ui| {
                egui::Grid::new("settings_grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("Backup directory:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.settings_dir)
                                .desired_width(300.0)
                                .hint_text("/path/to/channels"),
                        );
                        ui.end_row();

                        ui.label("Player command:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.player.command)
                                .desired_width(300.0)
                                .hint_text("mpv"),
                        );
                        ui.end_row();

                        ui.label("Cookie browser:");
                        let current_browser = self.config.player.browser.clone();
                        let display = BROWSERS
                            .iter()
                            .find(|(id, _)| *id == current_browser)
                            .map(|(_, label)| *label)
                            .unwrap_or(current_browser.as_str());
                        egui::ComboBox::from_id_salt("browser_combo")
                            .selected_text(display)
                            .show_ui(ui, |ui| {
                                for (id, label) in BROWSERS {
                                    if ui
                                        .selectable_label(self.config.player.browser == *id, *label)
                                        .clicked()
                                    {
                                        self.config.player.browser = id.to_string();
                                        self.downloader.browser = id.to_string();
                                    }
                                }
                            });
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

                        ui.label("Auto-check channels:");
                        ui.checkbox(&mut self.config.scheduler.enabled, "enabled");
                        ui.end_row();

                        ui.label("Check interval (hours):");
                        ui.add(
                            egui::DragValue::new(&mut self.config.scheduler.interval_hours)
                                .range(1..=168)
                                .suffix("h"),
                        );
                        ui.end_row();

                        ui.label("Web UI port:");
                        ui.add(
                            egui::DragValue::new(&mut self.config.web.port)
                                .range(1024..=65535),
                        );
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
                    ui.label(
                        egui::RichText::new("Theme previews immediately; other changes apply on save.")
                            .weak()
                            .small(),
                    );
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

        let is_watched = self.watched.contains(&selected_id);
        let resume_pos = self.resume_positions.get(&selected_id).copied();

        egui::TopBottomPanel::bottom("detail")
            .resizable(true)
            .default_height(220.0)
            .min_height(80.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.heading(&video.title);

                    if video.video_path.is_some() {
                        if ui.button("▶ Play").clicked() {
                            if let Some(p) = video.video_path.clone() {
                                self.play_with_tracking(&p, selected_id.clone());
                            }
                        }
                        if let Some(pos) = resume_pos {
                            if pos > 5.0 {
                                let label = format!("⏩ Resume ({})", format_duration(pos));
                                if ui.button(label).clicked() {
                                    if let Some(p) = video.video_path.clone() {
                                        self.play_with_tracking(&p, selected_id.clone());
                                    }
                                }
                                if ui.small_button("✖ clear position").clicked() {
                                    let _ = self.db.clear_position(&selected_id);
                                    self.resume_positions.remove(&selected_id);
                                }
                            }
                        }
                    }

                    if let Some(p) = video.video_path.clone() {
                        if ui.button("📁 Show file").clicked() {
                            self.open_in_file_manager(&p);
                        }
                    }

                    let watched_label = if is_watched { "✓ Watched" } else { "○ Mark watched" };
                    if ui.button(watched_label).clicked() {
                        self.toggle_watched(&selected_id);
                    }

                    if video.has_live_chat {
                        ui.label(egui::RichText::new("💬 live chat").small().weak());
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("✖ close").clicked() {
                            self.selected_video = None;
                        }
                        ui.label(
                            egui::RichText::new(format!("id: {}", video.id))
                                .monospace()
                                .weak(),
                        );
                    });
                });

                ui.horizontal(|ui| {
                    if let Some(secs) = video.duration_secs {
                        ui.label(egui::RichText::new(format_duration(secs)).small().weak());
                        ui.label(egui::RichText::new("·").weak());
                    }
                    if let Some(bytes) = video.file_size {
                        ui.label(egui::RichText::new(format_size(bytes)).small().weak());
                    }
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
        let show_channel = !matches!(self.sidebar_view, SidebarView::Channel(_) | SidebarView::Playlist(_, _));

        // Channel metadata banner
        if let SidebarView::Channel(ci) = self.sidebar_view {
            if let Some(ch) = self.library.get(ci) {
                if let Some(meta) = &ch.meta {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            if let Some(name) = &meta.uploader {
                                ui.strong(name);
                            }
                            if let Some(subs) = meta.subscriber_count {
                                ui.label(
                                    egui::RichText::new(format!("{} subscribers", format_subs(subs)))
                                        .weak(),
                                );
                            }
                            if let Some(url) = &meta.channel_url {
                                ui.hyperlink_to("Open on YouTube", url);
                            }
                        });
                    });
                }
            }
        }

        // Bulk mode toolbar
        ui.horizontal(|ui| {
            let label_text = if self.bulk_mode {
                format!("{} videos", cards.len())
            } else {
                format!("{} videos", cards.len())
            };
            ui.label(label_text);

            if !self.search.trim().is_empty() {
                ui.label(
                    egui::RichText::new(format!("(filtered by \"{}\")", self.search.trim())).weak(),
                );
            }

            ui.separator();

            if ui.selectable_label(self.bulk_mode, "☑ Select").clicked() {
                self.bulk_mode = !self.bulk_mode;
                if !self.bulk_mode {
                    self.bulk_selected.clear();
                }
            }

            if self.bulk_mode && !self.bulk_selected.is_empty() {
                ui.separator();
                let n = self.bulk_selected.len();
                ui.label(format!("{n} selected"));
                if ui.button("✓ Mark watched").clicked() {
                    self.bulk_mark_watched(true);
                    self.bulk_mode = false;
                }
                if ui.button("○ Mark unwatched").clicked() {
                    self.bulk_mark_watched(false);
                    self.bulk_mode = false;
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.selectable_value(&mut self.sort_mode, SortMode::SizeDesc, "Size ↓");
                ui.selectable_value(&mut self.sort_mode, SortMode::SizeAsc, "Size ↑");
                ui.selectable_value(&mut self.sort_mode, SortMode::DurationDesc, "Dur ↓");
                ui.selectable_value(&mut self.sort_mode, SortMode::DurationAsc, "Dur ↑");
                ui.selectable_value(&mut self.sort_mode, SortMode::Title, "Title");
                ui.label(egui::RichText::new("Sort:").weak());
            });
        });

        ui.separator();

        if cards.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("Nothing here.").weak());
                if !matches!(self.sidebar_view, SidebarView::ContinueWatching) {
                    ui.label(
                        egui::RichText::new(
                            "Drop a yt-dlp download into channels/<name>/, or use the Downloads panel.",
                        )
                        .small()
                        .weak(),
                    );
                }
            });
            return;
        }

        let density = self.card_density;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let thumb_w = (176.0 * density).round();
            let thumb_h = (99.0 * density).round();
            let thumb_size = egui::vec2(thumb_w, thumb_h);

            for card in &cards {
                let selected = self.selected_video.as_deref() == Some(card.id.as_str());
                let is_playing = self.currently_playing.as_deref() == Some(card.id.as_str());
                let bulk_checked = self.bulk_selected.contains(&card.id);
                let mut clicked_card = false;
                let mut play_card = false;
                let mut toggle_watched_card = false;

                ui.horizontal(|ui| {
                    let (rect, resp) = ui.allocate_exact_size(thumb_size, egui::Sense::click());
                    let texture = card.thumb_path.as_ref().and_then(|p| self.texture(ctx, p));
                    match &texture {
                        Some(handle) => {
                            egui::Image::new(handle)
                                .maintain_aspect_ratio(true)
                                .paint_at(ui, rect);
                        }
                        None => {
                            ui.painter().rect_filled(rect, 4.0, egui::Color32::from_gray(38));
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "▶",
                                egui::FontId::proportional(26.0 * density),
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
                    if is_playing {
                        ui.painter().rect_stroke(
                            rect,
                            4.0,
                            egui::Stroke::new(2.0, egui::Color32::from_rgb(110, 200, 110)),
                        );
                    }
                    if bulk_checked {
                        ui.painter().rect_stroke(
                            rect,
                            4.0,
                            egui::Stroke::new(3.0, egui::Color32::from_rgb(180, 130, 240)),
                        );
                    }
                    if card.watched {
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                rect.min,
                                egui::vec2(rect.width(), rect.height() * 0.18),
                            ),
                            0.0,
                            egui::Color32::from_rgba_premultiplied(30, 140, 60, 200),
                        );
                        ui.painter().text(
                            rect.min + egui::vec2(4.0, 2.0),
                            egui::Align2::LEFT_TOP,
                            "✓ watched",
                            egui::FontId::proportional(10.0 * density),
                            egui::Color32::WHITE,
                        );
                    }

                    if resp.clicked() {
                        clicked_card = true;
                    }
                    if resp.double_clicked() {
                        play_card = true;
                    }

                    ui.vertical(|ui| {
                        let title_color = if card.watched {
                            ui.visuals().weak_text_color()
                        } else {
                            ui.visuals().text_color()
                        };
                        if ui
                            .selectable_label(
                                selected,
                                egui::RichText::new(&card.title)
                                    .strong()
                                    .color(title_color),
                            )
                            .clicked()
                        {
                            clicked_card = true;
                        }
                        ui.horizontal(|ui| {
                            if show_channel {
                                ui.label(egui::RichText::new(&card.channel_name).small().weak());
                                ui.label(egui::RichText::new("·").weak());
                            }
                            ui.label(egui::RichText::new(&card.id).small().monospace().weak());
                            if let Some(secs) = card.duration_secs {
                                ui.label(egui::RichText::new("·").weak());
                                ui.label(egui::RichText::new(format_duration(secs)).small().weak());
                            }
                            if let Some(bytes) = card.file_size {
                                ui.label(egui::RichText::new("·").weak());
                                ui.label(egui::RichText::new(format_size(bytes)).small().weak());
                            }
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
                            if self.bulk_mode {
                                let chk_label = if bulk_checked { "☑" } else { "☐" };
                                if ui.small_button(chk_label).clicked() {
                                    clicked_card = true; // handled below
                                }
                            } else {
                                if card.video_path.is_some() && ui.small_button("▶ Play").clicked() {
                                    play_card = true;
                                }
                                if let Some(pos) = card.resume_pos {
                                    if pos > 5.0 && ui.small_button(format!("⏩ {}", format_duration(pos))).clicked() {
                                        play_card = true;
                                    }
                                }
                                if ui.small_button("Details").clicked() {
                                    clicked_card = true;
                                }
                                let w_label = if card.watched { "✓" } else { "○" };
                                if ui.small_button(w_label).on_hover_text("Toggle watched").clicked() {
                                    toggle_watched_card = true;
                                }
                            }
                        });
                    });
                });

                if self.bulk_mode {
                    if clicked_card {
                        let id = card.id.clone();
                        if self.bulk_selected.contains(&id) {
                            self.bulk_selected.remove(&id);
                        } else {
                            self.bulk_selected.insert(id);
                        }
                    }
                } else if play_card {
                    if let Some(p) = card.video_path.clone() {
                        let id = card.id.clone();
                        self.play_with_tracking(&p, id);
                    }
                    self.selected_video = Some(card.id.clone());
                } else if clicked_card {
                    self.selected_video = Some(card.id.clone());
                }
                if toggle_watched_card {
                    let id = card.id.clone();
                    self.toggle_watched(&id);
                }

                ui.separator();
            }
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.downloader.poll();
        self.check_notifications();

        let any_running = self.downloader.any_running();
        // Auto-rescan when all downloads finish
        let was_running = self.prev_job_states.values().any(|&s| s == JobState::Running);
        if was_running && !any_running {
            self.rescan();
        }

        if any_running {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }

        // Scheduled channel checks
        if self.config.scheduler.enabled && !any_running {
            let interval = std::time::Duration::from_secs(
                self.config.scheduler.interval_hours as u64 * 3600,
            );
            let due = self.last_scheduled_check
                .map_or(true, |t| t.elapsed() >= interval);
            if due {
                self.last_scheduled_check = Some(Instant::now());
                self.run_scheduled_check();
            }
        }

        // Poll mpv position updates
        if let Some(rx) = &self.mpv_rx {
            while let Ok((video_id, pos)) = rx.try_recv() {
                let _ = self.db.set_position(&video_id, pos);
                self.resume_positions.insert(video_id, pos);
            }
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

#[cfg(unix)]
fn spawn_mpv_tracker(sock_path: String, video_id: String, tx: std::sync::mpsc::Sender<(String, f64)>) {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    let mut stream = None;
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(500));
        if let Ok(s) = UnixStream::connect(&sock_path) {
            stream = Some(s);
            break;
        }
    }
    let mut stream = match stream {
        Some(s) => s,
        None => return,
    };
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok();

    let mut buf = [0u8; 4096];
    loop {
        std::thread::sleep(Duration::from_secs(5));
        let query = b"{\"command\":[\"get_property\",\"time-pos\"]}\n";
        if stream.write_all(query).is_err() {
            break;
        }
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                for chunk in buf[..n].split(|&b| b == b'\n').filter(|c| !c.is_empty()) {
                    if let Ok(s) = std::str::from_utf8(chunk) {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(s) {
                            if let Some(pos) = val.get("data").and_then(|v| v.as_f64()) {
                                if tx.send((video_id.clone(), pos)).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => break,
        }
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

fn format_duration(secs: f64) -> String {
    let secs = secs as u64;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.0} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{:.0} KB", bytes as f64 / 1_024.0)
    }
}

fn format_subs(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
