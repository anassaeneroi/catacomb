use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use eframe::egui;

use crate::downloader::{Downloader, JobState};
use crate::library::{self, Channel, Video};

/// Flattened, cheap-to-clone view of one video for a single frame of rendering.
struct Card {
    channel_idx: usize,
    video_idx: usize,
    channel_name: String,
    title: String,
    id: String,
    video_path: Option<PathBuf>,
    thumb_path: Option<PathBuf>,
    has_live_chat: bool,
}

pub struct App {
    channels_root: PathBuf,
    library: Vec<Channel>,
    selected_channel: Option<usize>,
    selected_video: Option<(usize, usize)>,
    search: String,
    downloader: Downloader,
    show_downloads: bool,
    dl_url: String,
    dl_dir: String,
    /// Decoded thumbnails. `None` means "tried and failed / not loadable".
    textures: HashMap<PathBuf, Option<egui::TextureHandle>>,
    /// How many new thumbnails we're still allowed to decode this frame.
    decode_budget: u32,
    desc_cache: HashMap<PathBuf, String>,
    status: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let channels_root = cwd.join("channels");
        let _ = std::fs::create_dir_all(&channels_root);
        let library = library::scan_channels(&channels_root);
        let status = format!(
            "{} channels, {} videos",
            library.len(),
            library.iter().map(|c| c.videos.len()).sum::<usize>()
        );
        Self {
            channels_root: channels_root.clone(),
            library,
            selected_channel: None,
            selected_video: None,
            search: String::new(),
            downloader: Downloader::new(channels_root),
            show_downloads: false,
            dl_url: String::new(),
            dl_dir: String::new(),
            textures: HashMap::new(),
            decode_budget: 0,
            desc_cache: HashMap::new(),
            status,
        }
    }

    fn rescan(&mut self) {
        self.library = library::scan_channels(&self.channels_root);
        self.selected_channel = None;
        self.selected_video = None;
        self.desc_cache.clear();
        self.status = format!(
            "Rescanned: {} channels, {} videos",
            self.library.len(),
            self.library.iter().map(|c| c.videos.len()).sum::<usize>()
        );
    }

    fn cards(&self) -> Vec<Card> {
        let query = self.search.trim().to_lowercase();
        let mut cards = Vec::new();
        for (ci, channel) in self.library.iter().enumerate() {
            if let Some(sel) = self.selected_channel {
                if sel != ci {
                    continue;
                }
            }
            for (vi, v) in channel.videos.iter().enumerate() {
                if !query.is_empty()
                    && !v.title.to_lowercase().contains(&query)
                    && !v.id.to_lowercase().contains(&query)
                {
                    continue;
                }
                cards.push(Card {
                    channel_idx: ci,
                    video_idx: vi,
                    channel_name: channel.name.clone(),
                    title: v.title.clone(),
                    id: v.id.clone(),
                    video_path: v.video_path.clone(),
                    thumb_path: v.thumb_path.clone(),
                    has_live_chat: v.has_live_chat,
                });
            }
        }
        cards
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
        match Command::new("mpv").arg(path).spawn() {
            Ok(_) => self.status = format!("Playing {}", file_label(path)),
            Err(_) => match Command::new("xdg-open").arg(path).spawn() {
                Ok(_) => self.status = format!("Opened {} in default player", file_label(path)),
                Err(e) => self.status = format!("Couldn't open {}: {e}", file_label(path)),
            },
        }
    }

    fn open_in_file_manager(&mut self, path: &Path) {
        let target = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        match Command::new("xdg-open").arg(target).spawn() {
            Ok(_) => {}
            Err(e) => self.status = format!("Couldn't open folder: {e}"),
        }
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.heading("YouTube Backup");
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
                let dl_label = if self.show_downloads {
                    "⬇ Downloads ▸"
                } else {
                    "⬇ Downloads"
                };
                if ui.selectable_label(self.show_downloads, dl_label).clicked() {
                    self.show_downloads = !self.show_downloads;
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
                    let total: usize = self.library.iter().map(|c| c.videos.len()).sum();
                    if ui
                        .selectable_label(self.selected_channel.is_none(), format!("⊞ All  ({total})"))
                        .clicked()
                    {
                        self.selected_channel = None;
                        self.selected_video = None;
                    }
                    ui.separator();
                    for (i, channel) in self.library.iter().enumerate() {
                        let label = format!("{}  ({})", channel.name, channel.videos.len());
                        if ui
                            .selectable_label(self.selected_channel == Some(i), label)
                            .on_hover_text(channel.path.display().to_string())
                            .clicked()
                        {
                            self.selected_channel = Some(i);
                            self.selected_video = None;
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
                ui.horizontal(|ui| {
                    ui.label("Save into  channels/");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.dl_dir)
                            .hint_text("folder_name")
                            .desired_width(170.0),
                    );
                });
                let ready = !self.dl_url.trim().is_empty() && !self.dl_dir.trim().is_empty();
                if ui
                    .add_enabled(ready, egui::Button::new("⬇  Start download"))
                    .clicked()
                {
                    let url = self.dl_url.trim().to_string();
                    let dir = self.dl_dir.trim().to_string();
                    self.downloader.start(url, dir.clone());
                    self.status = format!("Downloading into channels/{dir}");
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.heading("Jobs");
                    if !self.downloader.jobs.is_empty()
                        && !self.downloader.any_running()
                        && ui.button("Clear finished").clicked()
                    {
                        self.downloader
                            .jobs
                            .retain(|j| j.state == JobState::Running);
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
                                ui.label(format!("→ channels/{}", job.dir_name));
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
                                            ui.label(
                                                egui::RichText::new(line).small().monospace(),
                                            );
                                        }
                                    });
                            });
                        });
                    }
                });
            });
    }

    fn detail_panel(&mut self, ctx: &egui::Context) {
        let Some((ci, vi)) = self.selected_video else {
            return;
        };
        let Some(video) = self.library.get(ci).and_then(|c| c.videos.get(vi)).cloned() else {
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
                        ui.label(egui::RichText::new(format!("id: {}", video.id)).monospace().weak());
                        if ui.button("✖ close").clicked() {
                            self.selected_video = None;
                        }
                    });
                });
                ui.separator();
                let description = self.description(&video);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
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
                ui.label(egui::RichText::new(format!("(filtered by \"{}\")", self.search.trim())).weak());
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
                let selected = self.selected_video == Some((card.channel_idx, card.video_idx));
                let mut clicked_card = false;
                let mut play_card = false;
                ui.horizontal(|ui| {
                    let (rect, resp) = ui.allocate_exact_size(thumb_size, egui::Sense::click());
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
                            ui.painter().rect_filled(rect, 4.0, egui::Color32::from_gray(38));
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
                            .selectable_label(selected, egui::RichText::new(&card.title).strong())
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
                    self.selected_video = Some((card.channel_idx, card.video_idx));
                } else if clicked_card {
                    self.selected_video = Some((card.channel_idx, card.video_idx));
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
        // Decode at most a few thumbnails per frame so scrolling stays smooth.
        self.decode_budget = 6;

        self.top_bar(ctx);
        self.channel_panel(ctx);
        if self.show_downloads {
            self.downloads_panel(ctx);
        }
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
    Some(ctx.load_texture(
        path.to_string_lossy(),
        color_image,
        egui::TextureOptions::LINEAR,
    ))
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}
