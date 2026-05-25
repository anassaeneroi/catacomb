//! egui desktop application — the main window, sidebar, video grid, and settings UI.
//!
//! The [`App`] struct holds all UI state and implements [`eframe::App`].
//! Background work (downloads, scheduled rescans) runs in threads and
//! communicates back via `mpsc` channels stored on `App`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

use eframe::egui;

use crate::config::Config;
use crate::database::Database;
use crate::downloader::{DownloadQuality, Downloader, JobState};
use crate::platform::{self, classify_url, Platform, UrlKind};
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
    DateDesc,
    DateAsc,
}

#[derive(Clone, PartialEq)]
enum SidebarView {
    Channels,
    All,
    Channel(usize),
    Playlist(usize, usize),
    ContinueWatching,
    /// Activity feed — recent additions across all channels, sorted by mtime.
    Recent,
    Music,
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
    upload_date: Option<String>,
    mtime_unix: Option<u64>,
    watched: bool,
    resume_pos: Option<f64>,
}

pub struct App {
    config: Config,
    config_path: PathBuf,
    channels_root: PathBuf,
    /// Parent of `channels_root`. Owns every platform's sibling directory
    /// (`channels/`, `tiktok/`, …). Used as the maintenance scan root so
    /// non-YouTube content is included.
    library_root: PathBuf,
    library: Vec<library::Channel>,
    sidebar_view: SidebarView,
    selected_video: Option<String>,
    search: String,
    downloader: Downloader,
    show_downloads: bool,
    show_settings: bool,
    dl_url: String,
    dl_full_scan: bool,
    dl_quality: DownloadQuality,
    dl_music_mode: bool,
    /// Record an ongoing live stream from the start instead of joining live.
    dl_live: bool,
    /// For Twitch channel URLs: target the `/clips` listing instead of the
    /// default profile (VODs / past broadcasts). Has no effect on other
    /// platforms or on single-video URLs.
    dl_twitch_clips: bool,
    textures: HashMap<PathBuf, Option<egui::TextureHandle>>,
    thumb_request_tx: Sender<PathBuf>,
    thumb_result_rx: Receiver<(PathBuf, Option<egui::ColorImage>)>,
    thumb_pending: HashSet<PathBuf>,
    desc_cache: HashMap<PathBuf, String>,
    status: String,
    music_library: Vec<library::Track>,
    music_root: PathBuf,
    settings_dir: String,
    settings_plex_path: String,
    settings_source_url: String,
    plex_status: String,
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
    // Cards cache — recomputed only when inputs change
    cards_cache: Vec<Card>,
    cards_cache_key: Option<(String, SortMode, SidebarView, u64)>,
    library_generation: u64,
    // Web server
    web_server_running: bool,
    web_server_shutdown: Option<Sender<()>>,
    // Settings-window scratch state (not persisted directly)
    settings_bind_mode: String,
    settings_password_enabled: bool,
    settings_password_input: String,
    settings_cookies_input: String,
    settings_cookies_status: String,
    // File picker → chosen cookies.txt path arrives here from a dialog thread.
    cookies_pick_tx: Sender<PathBuf>,
    cookies_pick_rx: Receiver<PathBuf>,
    // Maintenance (library health) window
    show_maintenance: bool,
    health_report: Option<crate::maintenance::HealthReport>,
    // Statistics window
    show_stats: bool,
    stats_report: Option<crate::stats::StatsReport>,
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
        let library_root = channels_root
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| channels_root.clone());
        let _ = std::fs::create_dir_all(&library_root);
        // Pre-create every platform's folder so scans see them.
        for &p in Platform::all() {
            let dir = platform::platform_root(&channels_root, p);
            let _ = std::fs::create_dir_all(&dir);
        }
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

        let music_root = channels_root.with_file_name("music");
        let music_library = library::scan_music(&music_root);

        let max_concurrent = config.backup.max_concurrent;
        let use_bundled_ytdlp = config.backup.use_bundled_ytdlp;
        let browser = config.player.browser.clone();
        let config_bind = config.web.bind.clone();
        let password_set = db.get_setting("password_hash").ok().flatten().is_some();
        let plex_path_str = config.plex.library_path
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let source_url_str = config.web.source_url.clone().unwrap_or_default();

        let (cookies_pick_tx, cookies_pick_rx) = std::sync::mpsc::channel::<PathBuf>();

        let (thumb_request_tx, thumb_request_rx) = std::sync::mpsc::channel::<PathBuf>();
        let (thumb_result_tx, thumb_result_rx) =
            std::sync::mpsc::channel::<(PathBuf, Option<egui::ColorImage>)>();
        let ctx = cc.egui_ctx.clone();
        std::thread::spawn(move || {
            while let Ok(path) = thumb_request_rx.recv() {
                let img = decode_thumbnail_image(&path);
                if thumb_result_tx.send((path, img)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });

        Self {
            config,
            config_path,
            channels_root: channels_root.clone(),
            library_root,
            library,
            sidebar_view: SidebarView::All,
            selected_video: None,
            search: String::new(),
            downloader: Downloader::new(channels_root, browser, max_concurrent, use_bundled_ytdlp),
            show_downloads: false,
            show_settings: false,
            dl_url: String::new(),
            dl_full_scan: true,
            dl_quality: DownloadQuality::Best,
            dl_music_mode: false,
            dl_live: false,
            dl_twitch_clips: false,
            textures: HashMap::new(),
            thumb_request_tx,
            thumb_result_rx,
            thumb_pending: HashSet::new(),
            desc_cache: HashMap::new(),
            status,
            music_library,
            music_root,
            settings_dir,
            settings_plex_path: plex_path_str,
            settings_source_url: source_url_str,
            plex_status: String::new(),
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
            cards_cache: Vec::new(),
            cards_cache_key: None,
            library_generation: 0,
            web_server_running: false,
            web_server_shutdown: None,
            settings_bind_mode: crate::web::bind_mode_of(&config_bind).to_string(),
            settings_password_enabled: password_set,
            settings_password_input: String::new(),
            settings_cookies_input: String::new(),
            settings_cookies_status: String::new(),
            cookies_pick_tx,
            cookies_pick_rx,
            show_maintenance: false,
            health_report: None,
            show_stats: false,
            stats_report: None,
        }
    }

    fn rescan(&mut self) {
        self.library = library::scan_channels(&self.channels_root);
        self.music_library = library::scan_music(&self.music_root);
        self.sidebar_view = SidebarView::All;
        self.selected_video = None;
        self.desc_cache.clear();
        // Keep already-decoded thumbnail textures across rescans — they're keyed
        // by file path and don't change. Drop only entries whose thumbnail is no
        // longer in the library so removed videos don't leak GPU textures. This
        // avoids the full thumbnail reload flicker on every rescan.
        let valid: HashSet<PathBuf> = self
            .library
            .iter()
            .flat_map(|c| c.videos.iter().chain(c.playlists.iter().flat_map(|p| p.videos.iter())))
            .filter_map(|v| v.thumb_path.clone())
            .collect();
        self.textures.retain(|p, _| valid.contains(p));
        self.thumb_pending.retain(|p| valid.contains(p));
        // Bump the generation so the card cache recomputes against the new library.
        self.library_generation += 1;
        self.status = format!(
            "Rescanned: {} channels, {} videos",
            self.library.len(),
            self.library.iter().map(|c| c.total_videos()).sum::<usize>()
        );
    }

    fn cards_take(&mut self) -> Vec<Card> {
        let key = (
            self.search.clone(),
            self.sort_mode.clone(),
            self.sidebar_view.clone(),
            self.library_generation,
        );
        if self.cards_cache_key.as_ref() != Some(&key) {
            self.cards_cache = self.compute_cards();
            self.cards_cache_key = Some(key);
        }
        std::mem::take(&mut self.cards_cache)
    }

    fn compute_cards(&self) -> Vec<Card> {
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
                upload_date: v.upload_date.clone(),
                mtime_unix: v.mtime_unix,
                watched: self.watched.contains(&v.id),
                resume_pos,
            });
        };

        match &self.sidebar_view {
            SidebarView::Channels | SidebarView::Music => { return cards; } // rendered separately
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
            SidebarView::Recent => {
                // Most-recently-modified videos across the whole library.
                // Cap at 100 entries so the grid stays responsive for users
                // with thousands of files.
                for ch in &self.library {
                    for v in ch.videos.iter().chain(ch.playlists.iter().flat_map(|p| p.videos.iter())) {
                        if v.mtime_unix.is_some() {
                            add_video(&mut cards, &ch.name, v);
                        }
                    }
                }
                cards.sort_by(|a, b| b.mtime_unix.unwrap_or(0).cmp(&a.mtime_unix.unwrap_or(0)));
                cards.truncate(100);
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
            SortMode::DateDesc => {
                // Empty/missing dates sort to the end of "newest first".
                cards.sort_by(|a, b| b.upload_date.as_deref().unwrap_or("").cmp(a.upload_date.as_deref().unwrap_or("")));
            }
            SortMode::DateAsc => {
                // Empty/missing dates sort to the end of "oldest first" too.
                cards.sort_by(|a, b| {
                    match (a.upload_date.as_deref(), b.upload_date.as_deref()) {
                        (Some(x), Some(y)) => x.cmp(y),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    }
                });
            }
        }

        cards
    }

    fn find_video_by_id(&self, id: &str) -> Option<(Video, String)> {
        library::find_video(&self.library, id)
            .map(|(v, ch)| (v.clone(), ch.name.clone()))
    }

    fn texture(&mut self, _ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
        if let Some(slot) = self.textures.get(path) {
            return slot.clone();
        }
        let pb = path.to_path_buf();
        if self.thumb_pending.insert(pb.clone()) {
            let _ = self.thumb_request_tx.send(pb);
        }
        None
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
        // Only enable IPC for genuine mpv invocations — substring matching
        // would also fire for things like `mympv-wrapper`, `gnomempv`, etc.,
        // which don't implement the JSON-IPC protocol.
        let exe = std::path::Path::new(&cmd)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(cmd.as_str());
        let use_mpv_ipc = exe == "mpv" || exe == "mpv.exe";

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

    fn start_web_server(&mut self) {
        if self.web_server_running {
            return;
        }
        let shutdown = crate::web::run_with_shutdown(self.config.clone());
        self.web_server_shutdown = Some(shutdown);
        self.web_server_running = true;
        let port = self.config.web.port;
        self.status = format!("Web server started on port {port}. Access at http://localhost:{port}");
    }

    fn stop_web_server(&mut self) {
        if !self.web_server_running {
            return;
        }
        if let Some(shutdown) = self.web_server_shutdown.take() {
            let _ = shutdown.send(());
        }
        self.web_server_running = false;
        self.status = "Web server stopped.".to_string();
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
            .map(|ch| crate::downloader::recheck_url(ch))
            .collect();
        for url in urls {
            let info = classify_url(&url);
            // Scheduled re-check: never treat as live.
            self.downloader.start(url, &info, true, DownloadQuality::Best, false);
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
        ch.total_size_cached
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
                if ui.selectable_label(self.show_stats, "📊 Stats").clicked() {
                    self.show_stats = !self.show_stats;
                    if self.show_stats {
                        self.stats_report = Some(crate::stats::build(
                            &self.library,
                            &self.watched,
                            &self.resume_positions,
                            crate::stats::now_unix(),
                        ));
                    }
                }
                if ui.selectable_label(self.show_maintenance, "🩺 Maintenance").clicked() {
                    self.show_maintenance = !self.show_maintenance;
                    if self.show_maintenance {
                        self.health_report =
                            Some(crate::maintenance::scan(&self.library_root, &self.library));
                    }
                }
                if ui.selectable_label(self.show_settings, "⚙ Settings").clicked() {
                    self.show_settings = !self.show_settings;
                    if self.show_settings {
                        self.settings_dir = self.channels_root.display().to_string();
                        self.settings_plex_path = self.config.plex.library_path
                            .as_deref().map(|p| p.display().to_string()).unwrap_or_default();
                        self.settings_source_url = self.config.web.source_url.clone().unwrap_or_default();
                        self.plex_status.clear();
                        self.settings_bind_mode =
                            crate::web::bind_mode_of(&self.config.web.bind).to_string();
                        self.settings_password_enabled =
                            self.db.get_setting("password_hash").ok().flatten().is_some();
                        self.settings_password_input.clear();
                        self.settings_cookies_input.clear();
                        let (exists, n) = crate::web::cookies_status();
                        self.settings_cookies_status = if exists {
                            format!("{n} cookie(s) loaded")
                        } else {
                            "no cookies.txt".to_string()
                        };
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
                            self.sidebar_view == SidebarView::Channels,
                            format!("⊟ Channels  ({})", self.library.len()),
                        )
                        .clicked()
                    {
                        self.sidebar_view = SidebarView::Channels;
                        self.selected_video = None;
                    }

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

                    // Recent additions across the whole library — capped at 100
                    // in `compute_cards`. Only shown when the library has
                    // anything dated; on a brand-new install this stays hidden.
                    let has_dated = self.library.iter()
                        .flat_map(|c| c.all_videos())
                        .any(|v| v.mtime_unix.is_some());
                    if has_dated && ui
                        .selectable_label(
                            self.sidebar_view == SidebarView::Recent,
                            "🕒 Recent additions",
                        )
                        .clicked()
                    {
                        self.sidebar_view = SidebarView::Recent;
                        self.selected_video = None;
                    }

                    let music_count = self.music_library.len();
                    if ui
                        .selectable_label(
                            self.sidebar_view == SidebarView::Music,
                            format!("♫ Music  ({music_count})"),
                        )
                        .clicked()
                    {
                        self.sidebar_view = SidebarView::Music;
                        self.selected_video = None;
                    }

                    ui.separator();

                    // Collect any right-click download action outside the loop
                    let mut pending_ch_download: Option<(String, String)> = None; // (url, channel_name)

                    for i in 0..self.library.len() {
                        let (name, total, has_playlists, size_bytes, channel_url, platform) = {
                            let ch = &self.library[i];
                            // Prefer the stored `.source-url` over folder-name guessing so
                            // re-checks work across platforms (and across legacy YouTube
                            // libraries where the URL was never recorded).
                            let url = crate::downloader::recheck_url(ch);
                            (
                                ch.name.clone(),
                                ch.total_videos(),
                                !ch.playlists.is_empty(),
                                Self::channel_total_size(ch),
                                url,
                                ch.platform,
                            )
                        };

                        let is_ch_selected = matches!(self.sidebar_view, SidebarView::Channel(ci) if ci == i)
                            || matches!(self.sidebar_view, SidebarView::Playlist(ci, _) if ci == i);

                        let size_str = if size_bytes > 0 {
                            format!(" · {}", format_size(size_bytes))
                        } else {
                            String::new()
                        };
                        // Show platform icon for non-YouTube channels so the sidebar
                        // reads as a unified library while still making the source
                        // obvious at a glance.
                        let prefix = if platform == Platform::YouTube {
                            String::new()
                        } else {
                            format!("{} ", platform.icon())
                        };
                        let label = format!("{prefix}{}  ({}{})", name, total, size_str);

                        let ch_selected_no_pl = matches!(self.sidebar_view, SidebarView::Channel(ci) if ci == i);
                        let resp = ui
                            .selectable_label(ch_selected_no_pl, label)
                            .on_hover_text(format!(
                                "{}\n{}",
                                self.library[i].path.display(),
                                platform.display_name(),
                            ));
                        if resp.clicked() {
                            self.sidebar_view = SidebarView::Channel(i);
                            self.selected_video = None;
                        }
                        // Right-click context menu
                        let url_for_menu = channel_url.clone();
                        let name_for_menu = name.clone();
                        resp.context_menu(|ui| {
                            {
                                let url = &url_for_menu;
                                if ui.button("⬇ Check for new videos").clicked() {
                                    pending_ch_download = Some((url.clone(), name_for_menu.clone()));
                                    ui.close_menu();
                                }
                            }
                            if ui.button("📁 Open folder").clicked() {
                                let path = self.library[i].path.clone();
                                if let Err(e) = std::process::Command::new("xdg-open").arg(&path).spawn() {
                                    eprintln!("xdg-open: {e}");
                                }
                                ui.close_menu();
                            }
                        });

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

                    // Process deferred right-click download action
                    if let Some((url, ch_name)) = pending_ch_download {
                        let info = classify_url(&url);
                        self.downloader.start(url, &info, !self.dl_full_scan, DownloadQuality::Best, false);
                        self.status = format!("Checking {} for new videos…", ch_name);
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

                let info = classify_url(self.dl_url.trim());
                let plat_dir = info.platform.dir_name();
                let (type_label, dest_preview) = match &info.kind {
                    UrlKind::Channel { handle } => {
                        ("Channel", format!("→ {plat_dir}/{handle}/"))
                    }
                    UrlKind::Playlist => ("Playlist", format!("→ {plat_dir}/<creator>/<playlist>/")),
                    UrlKind::Video => ("Video", format!("→ {plat_dir}/<creator>/")),
                    UrlKind::Unknown => ("—", String::new()),
                };

                if !self.dl_url.trim().is_empty() {
                    ui.horizontal(|ui| {
                        ui.label("Source:");
                        ui.strong(format!("{} {}", info.platform.icon(), info.platform.display_name()));
                        ui.separator();
                        ui.label("Type:");
                        ui.strong(type_label);
                    });
                    if !dest_preview.is_empty() {
                        ui.label(egui::RichText::new(&dest_preview).small().weak());
                    }
                }

                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.dl_music_mode, false, "🎬 Video");
                    ui.selectable_value(&mut self.dl_music_mode, true, "🎵 Music");
                });

                if self.dl_music_mode {
                    ui.label(egui::RichText::new("Audio-only — saves to music/ directory").small().weak());
                } else {
                    ui.horizontal(|ui| {
                        ui.label("Quality:");
                        egui::ComboBox::from_id_salt("dl_quality")
                            .selected_text(self.dl_quality.label())
                            .show_ui(ui, |ui| {
                                for &q in DownloadQuality::all() {
                                    ui.selectable_value(&mut self.dl_quality, q, q.label());
                                }
                            });
                    });
                    ui.checkbox(&mut self.dl_full_scan, "Fast mode (stop at first already-downloaded video)")
                        .on_hover_text("Faster for large channels but may miss new videos if gaps exist in the archive. Leave off to check every video.");
                    ui.checkbox(&mut self.dl_live, "🔴 Live stream (record from start)")
                        .on_hover_text(
                            "Use for Twitch/YouTube Live broadcasts. Adds --live-from-start \
                             so yt-dlp records from the beginning instead of joining mid-stream. \
                             Also waits if the stream hasn't begun yet.",
                        );
                    if info.platform == Platform::Twitch
                        && matches!(info.kind, UrlKind::Channel { .. })
                    {
                        ui.checkbox(&mut self.dl_twitch_clips, "Clips only (Twitch)")
                            .on_hover_text(
                                "Pull only the channel's Clips section instead of the \
                                 default VOD/highlights mix. Rewrites the URL to \
                                 twitch.tv/<channel>/clips before submitting.",
                            );
                    }
                }

                let ready = !self.dl_url.trim().is_empty();
                if ui.add_enabled(ready, egui::Button::new("⬇  Start download")).clicked() {
                    let mut url = self.dl_url.trim().to_string();
                    // Twitch clips-only: rewrite `twitch.tv/<user>` to
                    // `twitch.tv/<user>/clips`. yt-dlp's TwitchClips extractor
                    // handles the rest and we still classify it as Channel so
                    // the output folder is unchanged.
                    if self.dl_twitch_clips
                        && info.platform == Platform::Twitch
                        && matches!(info.kind, UrlKind::Channel { .. })
                        && !url.contains("/clips")
                    {
                        url = format!("{}/clips", url.trim_end_matches('/'));
                    }
                    let dest = dest_preview.clone();
                    if self.dl_music_mode {
                        self.downloader.start_music(url);
                        self.status = "Downloading music…".to_string();
                    } else {
                        self.downloader.start(url, &info, !self.dl_full_scan, self.dl_quality, self.dl_live);
                        self.status = format!("Downloading: {dest}");
                    }
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
                if self.downloader.jobs.is_empty() && self.downloader.pending_count() == 0 {
                    ui.label(egui::RichText::new("Nothing queued yet.").weak());
                }
                let pending_count = self.downloader.pending_count();
                if pending_count > 0 {
                    let max = self.downloader.max_concurrent;
                    ui.label(
                        egui::RichText::new(format!(
                            "⏳ {pending_count} queued (max {max} concurrent)"
                        ))
                        .small()
                        .weak(),
                    );
                    let snapshots = self.downloader.pending_snapshots();
                    for (label, _url) in &snapshots {
                        ui.label(egui::RichText::new(format!("  · {label}")).small().weak());
                    }
                }
                let mut remove_job: Option<usize> = None;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let n = self.downloader.jobs.len();
                    for i in (0..n).rev() {
                        let job = &self.downloader.jobs[i];
                        let (text, color) = match job.state {
                            JobState::Running => ("running", egui::Color32::from_rgb(230, 200, 60)),
                            JobState::Done => ("done", egui::Color32::from_rgb(110, 200, 110)),
                            JobState::Failed => ("failed", egui::Color32::from_rgb(220, 110, 110)),
                        };
                        let finished = job.state != JobState::Running;
                        ui.push_id(i, |ui| {
                            ui.group(|ui| {
                                ui.horizontal(|ui| {
                                    ui.colored_label(color, text);
                                    ui.label(&job.label);
                                    if finished {
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui.small_button("✕").clicked() {
                                                remove_job = Some(i);
                                            }
                                        });
                                    }
                                });
                                ui.label(egui::RichText::new(&job.url).small().weak());
                                if job.state == JobState::Running {
                                    ui.add(egui::ProgressBar::new(job.progress).show_percentage());
                                }
                                let last = job.log.back().map(String::as_str).unwrap_or("");
                                if !last.is_empty() {
                                    ui.label(egui::RichText::new(last).small().monospace());
                                }
                                ui.collapsing("output log", |ui| {
                                    egui::ScrollArea::vertical()
                                        .max_height(180.0)
                                        .auto_shrink([false, true])
                                        .stick_to_bottom(true)
                                        .show(ui, |ui| {
                                            for line in &self.downloader.jobs[i].log {
                                                ui.label(egui::RichText::new(line).small().monospace());
                                            }
                                        });
                                });
                            });
                        });
                    }
                });
                if let Some(i) = remove_job {
                    self.downloader.remove_job(i);
                }
            });
    }

    fn maintenance_window(&mut self, ctx: &egui::Context) {
        if !self.show_maintenance {
            return;
        }
        let mut open = self.show_maintenance;
        let report = self.health_report.clone().unwrap_or_default();

        // Actions are collected during rendering and applied after the closure
        // to avoid borrowing `self` while the report is borrowed immutably.
        let mut to_remove: Vec<PathBuf> = Vec::new();
        let mut to_repair: Vec<String> = Vec::new();
        let mut rescan_health = false;

        egui::Window::new("🩺 Library health")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(620.0)
            .default_height(500.0)
            .show(ctx, |ui| {
                if ui.button("⟳ Rescan health").clicked() {
                    rescan_health = true;
                }
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading(format!("Duplicates ({})", report.duplicates.len()));
                    if report.duplicates.is_empty() {
                        ui.label(egui::RichText::new("No duplicate video IDs found.").weak());
                    }
                    for g in &report.duplicates {
                        ui.group(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{} [{}]", g.title, g.id)).strong(),
                            );
                            for c in &g.copies {
                                let size = c
                                    .file_size
                                    .map(format_size)
                                    .unwrap_or_else(|| "no video".to_string());
                                let tag = if c.recommended_keep { "✓ keep" } else { "✗ remove" };
                                ui.label(format!(
                                    "  {} · {} · {} files — {}",
                                    c.location, size, c.files.len(), tag
                                ));
                            }
                            if ui.button("🗑 Remove non-recommended copies").clicked() {
                                for c in &g.copies {
                                    if !c.recommended_keep {
                                        to_remove.extend(c.files.iter().cloned());
                                    }
                                }
                            }
                        });
                    }

                    ui.add_space(10.0);
                    ui.heading(format!("Missing assets ({})", report.missing.len()));
                    if report.missing.is_empty() {
                        ui.label(
                            egui::RichText::new(
                                "Every video has its thumbnail, metadata, and description.",
                            )
                            .weak(),
                        );
                    }
                    if report.missing.len() > 1
                        && ui
                            .button(format!("⬇ Fetch all missing ({})", report.missing.len()))
                            .clicked()
                    {
                        for m in &report.missing {
                            to_repair.push(m.id.clone());
                        }
                    }
                    for m in &report.missing {
                        ui.horizontal(|ui| {
                            let need: Vec<&str> = [
                                m.missing_thumbnail.then_some("thumbnail"),
                                m.missing_info.then_some("metadata"),
                                m.missing_description.then_some("description"),
                            ]
                            .into_iter()
                            .flatten()
                            .collect();
                            ui.label(format!("{} — missing {}", m.title, need.join(", ")));
                            if ui.button("⬇ Fetch").clicked() {
                                to_repair.push(m.id.clone());
                            }
                        });
                    }
                });
            });
        self.show_maintenance = open;

        // ── Apply collected actions ────────────────────────────────────────
        let mut changed = false;
        if !to_remove.is_empty() {
            let (removed, errors) =
                crate::maintenance::remove_files(&self.library_root, &to_remove);
            self.status = if errors.is_empty() {
                format!("Removed {removed} file(s)")
            } else {
                format!("Removed {removed} file(s), {} error(s)", errors.len())
            };
            changed = true;
        }
        if !to_repair.is_empty() {
            for id in &to_repair {
                if let Some((dir, stem)) = crate::maintenance::locate(&self.library, id) {
                    self.downloader.repair(id, &dir, &stem);
                }
            }
            self.status = format!("Queued {} repair(s) — see Downloads", to_repair.len());
            self.show_downloads = true;
        }
        if changed || rescan_health {
            self.rescan();
            self.health_report =
                Some(crate::maintenance::scan(&self.library_root, &self.library));
        }
    }

    fn stats_window(&mut self, ctx: &egui::Context) {
        if !self.show_stats { return; }
        let mut open = self.show_stats;
        let report = match &self.stats_report {
            Some(r) => r.clone(),
            None => return,
        };
        let mut rescan = false;
        egui::Window::new("📊 Library statistics")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .default_height(520.0)
            .show(ctx, |ui| {
                if ui.button("⟳ Recompute").clicked() { rescan = true; }
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading("Totals");
                    egui::Grid::new("stats_totals").num_columns(2).striped(true).show(ui, |ui| {
                        ui.label("Channels");      ui.label(report.total_channels.to_string()); ui.end_row();
                        ui.label("Videos");        ui.label(report.total_videos.to_string()); ui.end_row();
                        ui.label("Playlists");     ui.label(report.total_playlists.to_string()); ui.end_row();
                        ui.label("Disk used");     ui.label(format_size(report.total_size_bytes)); ui.end_row();
                        ui.label("Total runtime"); ui.label(format_hours(report.total_duration_secs)); ui.end_row();
                        ui.label("Watched");
                        ui.label(format!("{} · {}", report.watched_count, format_hours(report.watched_duration_secs)));
                        ui.end_row();
                        ui.label("Continue watching"); ui.label(report.continue_watching_count.to_string()); ui.end_row();
                    });

                    ui.add_space(8.0);
                    ui.heading(format!("Downloads — last {} weeks", report.downloads_per_week.len()));
                    draw_bars(ui, report.downloads_per_week.iter().map(|w| (
                        format!("{}", week_label(w.week_start_unix)),
                        w.count as f32,
                        format!("{} videos · {}", w.count, format_size(w.size_bytes)),
                    )));

                    if !report.videos_per_year.is_empty() {
                        ui.add_space(8.0);
                        ui.heading("Videos by upload year");
                        draw_bars(ui, report.videos_per_year.iter().map(|y| (
                            y.year.to_string(),
                            y.count as f32,
                            format!("{}: {}", y.year, y.count),
                        )));
                    }

                    ui.add_space(8.0);
                    ui.heading("Top channels by size");
                    for row in &report.top_channels_by_size {
                        ui.label(format!(
                            "  {} — {} videos · {} · {}",
                            row.name, row.count, format_size(row.size_bytes),
                            format_hours(row.duration_secs),
                        ));
                    }

                    ui.add_space(8.0);
                    ui.heading("Top channels by count");
                    for row in &report.top_channels_by_count {
                        ui.label(format!(
                            "  {} — {} videos · {} · {}",
                            row.name, row.count, format_size(row.size_bytes),
                            format_hours(row.duration_secs),
                        ));
                    }
                });
            });
        self.show_stats = open;
        if rescan {
            self.stats_report = Some(crate::stats::build(
                &self.library, &self.watched, &self.resume_positions, crate::stats::now_unix(),
            ));
        }
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

                        ui.label("Max concurrent downloads:");
                        ui.add(
                            egui::DragValue::new(&mut self.config.backup.max_concurrent)
                                .range(1..=10),
                        )
                        .on_hover_text("Maximum simultaneous yt-dlp processes. Extra downloads queue automatically.");
                        ui.end_row();

                        ui.label("yt-dlp binary:");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut self.config.backup.use_bundled_ytdlp, false, "System")
                                .on_hover_text("Use whatever yt-dlp is on PATH.");
                            ui.radio_value(&mut self.config.backup.use_bundled_ytdlp, true, "Bundled")
                                .on_hover_text("Use the yt-dlp + deno installed under ~/.local/share/yt-offline/bin/.");
                            let installed = crate::ytdlp_bin::bundled_installed();
                            let btn_label = if installed { "Update" } else { "Install" };
                            if ui.button(btn_label)
                                .on_hover_text("Download (or update) the bundled yt-dlp + deno from GitHub. Streams output as a job.")
                                .clicked()
                            {
                                self.downloader.start_ytdlp_update();
                            }
                            if installed {
                                ui.label(egui::RichText::new("✓ installed").weak().small());
                            } else {
                                ui.label(egui::RichText::new("not installed").weak().small());
                            }
                        });
                        ui.end_row();

                        ui.label("Web UI port:");
                        ui.add(
                            egui::DragValue::new(&mut self.config.web.port)
                                .range(1024..=65535),
                        );
                        ui.end_row();

                        ui.label("Bind interface:");
                        let binds = crate::web::get_available_binds(self.config.web.port);
                        let selected_label = binds
                            .iter()
                            .find(|b| b.id == self.settings_bind_mode)
                            .map(|b| b.label.clone())
                            .unwrap_or_else(|| "Localhost only".to_string());
                        egui::ComboBox::from_id_salt("bind_combo")
                            .selected_text(selected_label)
                            .show_ui(ui, |ui| {
                                for b in &binds {
                                    if ui
                                        .selectable_label(self.settings_bind_mode == b.id, &b.label)
                                        .clicked()
                                    {
                                        self.settings_bind_mode = b.id.clone();
                                    }
                                }
                            });
                        ui.end_row();

                        ui.label("Download password:");
                        ui.vertical(|ui| {
                            ui.checkbox(&mut self.settings_password_enabled, "require for web downloads");
                            if self.settings_password_enabled {
                                let hint = if self.settings_password_input.is_empty() {
                                    "leave blank to keep current"
                                } else {
                                    "set a password"
                                };
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.settings_password_input)
                                        .password(true)
                                        .desired_width(220.0)
                                        .hint_text(hint),
                                );
                            }
                        });
                        ui.end_row();

                        ui.label("Web server:");
                        ui.horizontal(|ui| {
                            if self.web_server_running {
                                if ui.button("🛑 Stop").clicked() {
                                    self.stop_web_server();
                                }
                                ui.label(egui::RichText::new("Running").small().weak());
                            } else {
                                if ui.button("▶ Start").clicked() {
                                    self.start_web_server();
                                }
                                ui.label(egui::RichText::new("Stopped").small().weak());
                            }
                        });
                        ui.end_row();

                        ui.label("Cookies:");
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(&self.settings_cookies_status).small().weak());
                            if ui.button("📁 Choose cookies.txt…").clicked() {
                                // Run the file dialog off the UI thread; the chosen
                                // path comes back via cookies_pick_rx (polled in update()).
                                let tx = self.cookies_pick_tx.clone();
                                std::thread::spawn(move || {
                                    if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                                        .enable_all()
                                        .build()
                                    {
                                        let picked = rt.block_on(async {
                                            rfd::AsyncFileDialog::new()
                                                .set_title("Select cookies.txt")
                                                .add_filter("cookies", &["txt"])
                                                .pick_file()
                                                .await
                                        });
                                        if let Some(f) = picked {
                                            let _ = tx.send(f.path().to_path_buf());
                                        }
                                    }
                                });
                            }
                            ui.label(egui::RichText::new("…or paste below").small().weak());
                            ui.add(
                                egui::TextEdit::multiline(&mut self.settings_cookies_input)
                                    .desired_rows(3)
                                    .desired_width(300.0)
                                    .hint_text("paste Netscape cookies.txt…"),
                            );
                            ui.horizontal(|ui| {
                                if ui.button("Update cookies").clicked() {
                                    match crate::web::write_cookies(&self.settings_cookies_input) {
                                        Ok(n) => {
                                            self.settings_cookies_status = format!("{n} cookie(s) loaded");
                                            self.settings_cookies_input.clear();
                                            self.status = format!("Cookies updated ({n} entries)");
                                        }
                                        Err(e) => self.status = format!("Cookies error: {e}"),
                                    }
                                }
                                if ui.button("Clear cookies").clicked() {
                                    match crate::web::clear_cookies() {
                                        Ok(()) => {
                                            self.settings_cookies_status = "no cookies.txt".to_string();
                                            self.status = "Cookies cleared".to_string();
                                        }
                                        Err(e) => self.status = format!("Error clearing cookies: {e}"),
                                    }
                                }
                            });
                            ui.label(
                                egui::RichText::new("Export via a browser extension, then paste.")
                                    .small()
                                    .weak(),
                            );
                        });
                        ui.end_row();
                    });

                ui.add_space(8.0);
                ui.separator();
                ui.heading("Source code (AGPL §13)");
                ui.add_space(4.0);
                ui.label("Repository URL (shown in web UI footer):");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings_source_url)
                        .hint_text("https://codeberg.org/you/your-fork")
                        .desired_width(400.0),
                );
                ui.label(
                    egui::RichText::new(
                        "Every network user must be offered a way to obtain the running source code. \
                         Leave empty to hide the link.",
                    )
                    .small()
                    .weak(),
                );

                ui.add_space(8.0);
                ui.separator();
                ui.heading("Plex");
                ui.add_space(4.0);
                ui.label("Plex library path:");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings_plex_path)
                            .hint_text("/media/plex/YouTube")
                            .desired_width(300.0),
                    );
                });
                ui.label(
                    egui::RichText::new(
                        "Creates a TV-show symlink tree here. Point a Plex TV library at this folder.",
                    )
                    .small()
                    .weak(),
                );
                ui.horizontal(|ui| {
                    let can_generate = !self.settings_plex_path.trim().is_empty();
                    if ui.add_enabled(can_generate, egui::Button::new("⟳ Generate Plex library")).clicked() {
                        let plex_path = PathBuf::from(self.settings_plex_path.trim());
                        let result = crate::plex::generate(&self.library, &plex_path);
                        self.plex_status = if result.errors.is_empty() {
                            format!("{} link(s) created/updated", result.links_created)
                        } else {
                            format!(
                                "{} link(s) created, {} error(s): {}",
                                result.links_created,
                                result.errors.len(),
                                result.errors.first().unwrap_or(&String::new())
                            )
                        };
                    }
                    if !self.plex_status.is_empty() {
                        ui.label(egui::RichText::new(&self.plex_status).small());
                    }
                });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    if ui.button("Apply & Save").clicked() {
                        let new_dir = PathBuf::from(&self.settings_dir);
                        let dir_changed = new_dir != self.config.backup.directory;
                        self.config.backup.directory = new_dir.clone();

                        // Plex library path
                        let plex_trimmed = self.settings_plex_path.trim();
                        self.config.plex.library_path = if plex_trimmed.is_empty() {
                            None
                        } else {
                            Some(PathBuf::from(plex_trimmed))
                        };

                        // Source URL (AGPL §13)
                        let src_trimmed = self.settings_source_url.trim();
                        self.config.web.source_url = if src_trimmed.is_empty() {
                            None
                        } else {
                            Some(src_trimmed.to_string())
                        };

                        // Resolve the chosen interface to a concrete bind address.
                        let new_bind = crate::web::resolve_bind_mode(&self.settings_bind_mode);
                        let bind_changed = new_bind != self.config.web.bind;
                        self.config.web.bind = new_bind;

                        // Apply the download-password setting to the database.
                        let pwd_result = if !self.settings_password_enabled {
                            self.db.set_setting("password_hash", None)
                        } else if !self.settings_password_input.is_empty() {
                            match crate::web::hash_password(&self.settings_password_input) {
                                Some(hash) => {
                                    let r = self.db.set_setting("password_hash", Some(&hash));
                                    self.settings_password_input.clear();
                                    r
                                }
                                None => {
                                    self.status = "Error hashing password".to_string();
                                    Ok(())
                                }
                            }
                        } else {
                            Ok(())
                        };
                        if let Err(e) = pwd_result {
                            self.status = format!("DB error: {e}");
                        }

                        match self.config.save(&self.config_path) {
                            Ok(_) => self.status = "Settings saved.".to_string(),
                            Err(e) => self.status = format!("Error saving config: {e}"),
                        }
                        self.downloader.max_concurrent = self.config.backup.max_concurrent;
                        self.downloader.use_bundled_ytdlp = self.config.backup.use_bundled_ytdlp;
                        if dir_changed {
                            self.channels_root = new_dir.clone();
                            self.library_root = new_dir
                                .parent()
                                .map(|p| p.to_path_buf())
                                .unwrap_or_else(|| new_dir.clone());
                            self.downloader.channels_root = new_dir;
                            let _ = std::fs::create_dir_all(&self.channels_root);
                            let _ = std::fs::create_dir_all(&self.library_root);
                            for &p in Platform::all() {
                                let _ = std::fs::create_dir_all(platform::platform_root(&self.channels_root, p));
                            }
                            self.rescan();
                        }
                        // Re-bind a running server so the new interface takes effect now.
                        if bind_changed && self.web_server_running {
                            self.stop_web_server();
                            self.start_web_server();
                            self.status = "Settings saved. Web server re-bound.".to_string();
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
                    if let Some(date) = video.upload_date.as_deref().map(format_upload_date) {
                        if !date.is_empty() {
                            ui.label(
                                egui::RichText::new(format!("📅 {date}"))
                                    .small()
                                    .weak(),
                            );
                            ui.label(egui::RichText::new("·").weak());
                        }
                    }
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

    fn channel_grid(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let density = self.card_density;
        let thumb_w = (176.0 * density).round();
        let thumb_h = (99.0 * density).round();
        let card_w = thumb_w + 4.0; // 2px border each side

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let available_w = ui.available_width();
            let cols = ((available_w / (card_w + 12.0)) as usize).max(1);
            let n = self.library.len();

            for row_start in (0..n).step_by(cols) {
                ui.horizontal(|ui| {
                    for i in row_start..(row_start + cols).min(n) {
                        // Collect data we need without holding a borrow into self.library
                        let (name, total, size_bytes, thumb_path) = {
                            let ch = &self.library[i];
                            let thumb = ch.videos.iter()
                                .chain(ch.playlists.iter().flat_map(|p| p.videos.iter()))
                                .find_map(|v| v.thumb_path.clone());
                            (ch.name.clone(), ch.total_videos(), ch.total_size_cached, thumb)
                        };

                        ui.push_id(i, |ui| {
                            let (card_rect, card_resp) = ui.allocate_exact_size(
                                egui::vec2(card_w, thumb_h + 46.0 * density),
                                egui::Sense::click(),
                            );
                            let visuals = ui.visuals();
                            let border_color = if card_resp.hovered() {
                                visuals.selection.bg_fill
                            } else {
                                visuals.widgets.noninteractive.bg_stroke.color
                            };
                            ui.painter().rect_stroke(card_rect, 6.0, egui::Stroke::new(2.0, border_color));

                            let thumb_rect = egui::Rect::from_min_size(
                                card_rect.min + egui::vec2(2.0, 2.0),
                                egui::vec2(thumb_w, thumb_h),
                            );
                            let texture = thumb_path.as_ref().and_then(|p| self.texture(ctx, p));
                            match &texture {
                                Some(handle) => {
                                    egui::Image::new(handle)
                                        .maintain_aspect_ratio(true)
                                        .paint_at(ui, thumb_rect);
                                }
                                None => {
                                    ui.painter().rect_filled(thumb_rect, 4.0, egui::Color32::from_gray(30));
                                    ui.painter().text(
                                        thumb_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        "📺",
                                        egui::FontId::proportional(28.0 * density),
                                        egui::Color32::from_gray(100),
                                    );
                                }
                            }

                            let text_top = card_rect.min + egui::vec2(6.0, thumb_h + 6.0);
                            ui.painter().text(
                                text_top,
                                egui::Align2::LEFT_TOP,
                                &name,
                                egui::FontId::proportional(13.0 * density),
                                visuals.text_color(),
                            );
                            let sub = format!(
                                "{} video{}{}",
                                total,
                                if total == 1 { "" } else { "s" },
                                if size_bytes > 0 { format!(" · {}", format_size(size_bytes)) } else { String::new() }
                            );
                            ui.painter().text(
                                text_top + egui::vec2(0.0, 16.0 * density),
                                egui::Align2::LEFT_TOP,
                                &sub,
                                egui::FontId::proportional(11.0 * density),
                                visuals.weak_text_color(),
                            );

                            if card_resp.clicked() {
                                self.sidebar_view = SidebarView::Channel(i);
                                self.selected_video = None;
                            }
                        });
                        ui.add_space(8.0);
                    }
                });
                ui.add_space(8.0);
            }
        });
    }

    fn music_view(&mut self, ui: &mut egui::Ui) {
        ui.heading("Music");
        if self.music_library.is_empty() {
            ui.label(egui::RichText::new(
                "No tracks yet. Download audio with Music mode in the Downloads panel."
            ).weak());
            return;
        }
        let mut current_artist = String::new();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for track in &self.music_library {
                if track.artist != current_artist {
                    current_artist = track.artist.clone();
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(&track.artist).strong());
                    ui.separator();
                }
                ui.push_id(&track.id, |ui| {
                    ui.horizontal(|ui| {
                        let dur = track.duration_secs
                            .map(|s| {
                                let m = s as u64 / 60;
                                let sec = s as u64 % 60;
                                format!("{m}:{sec:02}")
                            })
                            .unwrap_or_default();
                        if ui.selectable_label(false, &track.title).clicked() {
                            if let Err(e) = Command::new(&self.config.player.command)
                                .arg(&track.path)
                                .spawn()
                            {
                                self.status = format!("Could not open player: {e}");
                            }
                        }
                        if !dur.is_empty() {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(egui::RichText::new(dur).small().weak());
                            });
                        }
                    });
                });
            }
        });
    }

    fn video_list(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        if self.sidebar_view == SidebarView::Channels {
            self.channel_grid(ctx, ui);
            return;
        }
        if self.sidebar_view == SidebarView::Music {
            self.music_view(ui);
            return;
        }
        let cards = self.cards_take();
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
                ui.selectable_value(&mut self.sort_mode, SortMode::DateDesc, "Newest");
                ui.selectable_value(&mut self.sort_mode, SortMode::DateAsc, "Oldest");
                ui.selectable_value(&mut self.sort_mode, SortMode::SizeDesc, "Largest");
                ui.selectable_value(&mut self.sort_mode, SortMode::SizeAsc, "Smallest");
                ui.selectable_value(&mut self.sort_mode, SortMode::DurationDesc, "Longest");
                ui.selectable_value(&mut self.sort_mode, SortMode::DurationAsc, "Shortest");
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
                            if let Some(date) = card.upload_date.as_deref().map(format_upload_date) {
                                if !date.is_empty() {
                                    ui.label(egui::RichText::new("·").weak());
                                    ui.label(egui::RichText::new(date).small().weak());
                                }
                            }
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
        self.cards_cache = cards;
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.downloader.poll();
        // Snapshot the previous-frame running state BEFORE check_notifications
        // overwrites prev_job_states with the current frame's. Otherwise
        // was_running tracks the wrong frame and the auto-rescan never fires.
        let was_running_prev = self.prev_job_states.values().any(|&s| s == JobState::Running);
        self.check_notifications();

        let any_running = self.downloader.any_running();
        if was_running_prev && !any_running {
            self.rescan();
        }

        if any_running {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }

        // Scheduled channel checks
        if self.config.scheduler.enabled && !any_running {
            // Defensive clamp: a manually-edited config.toml with 0 hours
            // would otherwise re-fire every frame.
            let hours = self.config.scheduler.interval_hours.max(1);
            let interval = std::time::Duration::from_secs(hours as u64 * 3600);
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

        // Drain any decoded thumbnails from the worker thread
        while let Ok((path, img)) = self.thumb_result_rx.try_recv() {
            self.thumb_pending.remove(&path);
            let handle = img.map(|color_image| {
                ctx.load_texture(
                    path.to_string_lossy(),
                    color_image,
                    egui::TextureOptions::LINEAR,
                )
            });
            self.textures.insert(path, handle);
        }

        // A cookies.txt was chosen in the file dialog — validate and install it.
        while let Ok(path) = self.cookies_pick_rx.try_recv() {
            match std::fs::read_to_string(&path) {
                Ok(content) => match crate::web::write_cookies(&content) {
                    Ok(n) => {
                        self.settings_cookies_status = format!("{n} cookie(s) loaded");
                        self.status = format!("Cookies imported ({n} entries)");
                    }
                    Err(e) => self.status = format!("Cookies error: {e}"),
                },
                Err(e) => self.status = format!("Could not read {}: {e}", path.display()),
            }
        }

        self.top_bar(ctx);
        self.channel_panel(ctx);
        if self.show_downloads {
            self.downloads_panel(ctx);
        }
        self.settings_window(ctx);
        self.maintenance_window(ctx);
        self.stats_window(ctx);
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

fn decode_thumbnail_image(path: &Path) -> Option<egui::ColorImage> {
    let image = image::open(path).ok()?;
    let image = image.thumbnail(384, 216);
    let rgba = image.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    Some(egui::ColorImage::from_rgba_unmultiplied([w, h], rgba.as_raw()))
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

/// Convert yt-dlp's native `YYYYMMDD` upload-date string into `YYYY-MM-DD`
/// for display. Returns an empty string when input doesn't look like a date
/// so callers can decide whether to skip the row entirely.
fn format_upload_date(yyyymmdd: &str) -> String {
    if yyyymmdd.len() < 8 || !yyyymmdd.chars().all(|c| c.is_ascii_digit()) {
        return String::new();
    }
    format!("{}-{}-{}", &yyyymmdd[..4], &yyyymmdd[4..6], &yyyymmdd[6..8])
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

/// Format a number of seconds as `Hh Mm` (or `Mm` under 1 hour, `0h` empty).
fn format_hours(secs: f64) -> String {
    if secs < 60.0 { return "0h".to_string(); }
    let h = (secs / 3600.0) as u64;
    let m = ((secs as u64 % 3600) / 60) as u64;
    if h > 0 { format!("{h}h {m}m") } else { format!("{m}m") }
}

/// Format the week-start unix time as `M/D` for an axis label.
fn week_label(unix: u64) -> String {
    // Reproduce just enough calendar math to render M/D without pulling in a
    // chrono dep: count days since 1970-01-01, then walk the year/month table.
    let days = (unix / 86_400) as i64;
    let (mut y, mut d) = (1970i64, days);
    loop {
        let ly = is_leap(y as u32);
        let yd = if ly { 366 } else { 365 };
        if d < yd { break; }
        d -= yd; y += 1;
    }
    let months = if is_leap(y as u32) {
        [31,29,31,30,31,30,31,31,30,31,30,31]
    } else {
        [31,28,31,30,31,30,31,31,30,31,30,31]
    };
    let mut m = 0usize;
    while m < 12 && d >= months[m] as i64 { d -= months[m] as i64; m += 1; }
    format!("{}/{}", m as u32 + 1, d + 1)
}
fn is_leap(y: u32) -> bool { (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 }

/// Draw a simple bar chart of `(label, value, tooltip)` rows. Heights scale
/// to the max value in the iterator.
fn draw_bars<I>(ui: &mut egui::Ui, items: I)
where I: IntoIterator<Item = (String, f32, String)>,
{
    let rows: Vec<_> = items.into_iter().collect();
    if rows.is_empty() { return; }
    let max = rows.iter().map(|r| r.1).fold(1.0f32, f32::max);
    let chart_h = 70.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (label, value, tip) in &rows {
            ui.vertical(|ui| {
                let h = (value / max) * chart_h;
                let bar_w = 22.0;
                let (rect, resp) = ui.allocate_exact_size(
                    egui::vec2(bar_w, chart_h + 14.0),
                    egui::Sense::hover(),
                );
                let bar_rect = egui::Rect::from_min_max(
                    egui::pos2(rect.min.x, rect.max.y - 14.0 - h),
                    egui::pos2(rect.max.x, rect.max.y - 14.0),
                );
                ui.painter().rect_filled(
                    bar_rect, 2.0,
                    ui.visuals().selection.bg_fill,
                );
                ui.painter().text(
                    egui::pos2(rect.center().x, rect.max.y),
                    egui::Align2::CENTER_BOTTOM,
                    label,
                    egui::FontId::proportional(9.0),
                    ui.visuals().weak_text_color(),
                );
                resp.on_hover_text(tip);
            });
        }
    });
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
