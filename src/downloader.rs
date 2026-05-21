//! Running `yt-dlp` in the background and surfacing its progress to the UI.
//!
//! Each call to [`Downloader::start`] spawns a thread that runs `yt-dlp` and
//! pipes stdout/stderr back to the main thread through an `mpsc` channel.
//! The caller polls for updates via [`Downloader::poll`], which drains the
//! channel into each [`Job`]'s log buffer.
//!
//! # yt-dlp command flags used
//!
//! | Flag | Purpose |
//! |---|---|
//! | `--cookies cookies.txt` | Pass browser cookies for age-gated/member videos |
//! | `--write-subs --write-auto-subs` | Download subtitles alongside the video |
//! | `--write-thumbnail` | Download channel/video thumbnails |
//! | `--write-description` | Save video description as a sidecar `.description` file |
//! | `--write-info-json` | Save full metadata as a `.info.json` sidecar |
//! | `--remux-video mkv` | Re-container to MKV (no re-encode) |
//! | `--embed-metadata --embed-info-json --embed-chapters` | Embed rich metadata into the MKV |
//! | `--xattrs` | Store metadata in filesystem extended attributes |
//! | `--sponsorblock-mark all` | Mark (but don't remove) SponsorBlock segments |
//! | `--extractor-args youtube:player_client=web` | Use the web player API to avoid throttling |
//! | `--impersonate Chrome-146:Macos-26` | Impersonate a real browser for bot detection |
//! | `--break-on-existing` | Stop when the archive file records the video as already downloaded |
//! | `--download-archive archive.txt` | Record downloaded IDs to avoid re-downloading |

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::thread;

/// Describes the kind of YouTube URL being downloaded, which determines the
/// output path template passed to yt-dlp.
pub enum UrlKind {
    /// A channel URL (`/@handle`, `/channel/ID`, or `/c/name`).
    Channel { handle: String },
    Playlist,
    Video,
    Unknown,
}

/// Build a YouTube URL from a library folder name that yt-dlp will resolve to
/// the same folder it already downloaded to.
///
/// Folder names that look like a channel ID (`UC` + 22 chars) use the
/// `/channel/` form; everything else is treated as a handle and gets `/@`.
/// This avoids the mismatch where info.json's canonical `channel_url` field
/// points to `/channel/UCxxx` and yt-dlp then creates a second folder.
pub fn check_url_for_folder(folder_name: &str) -> String {
    if folder_name.starts_with("UC") && folder_name.len() == 24 {
        format!("https://www.youtube.com/channel/{folder_name}")
    } else {
        format!("https://www.youtube.com/@{folder_name}")
    }
}

/// Classify a YouTube URL into a [`UrlKind`] by inspecting its path.
pub fn detect_url_kind(url: &str) -> UrlKind {
    if url.contains("playlist?list=") {
        return UrlKind::Playlist;
    }
    if let Some(h) = extract_after(url, "/@") {
        return UrlKind::Channel { handle: h.to_string() };
    }
    if let Some(h) = extract_after(url, "/channel/") {
        return UrlKind::Channel { handle: h.to_string() };
    }
    if let Some(h) = extract_after(url, "/c/") {
        return UrlKind::Channel { handle: h.to_string() };
    }
    if url.contains("watch?v=") || url.contains("youtu.be/") {
        return UrlKind::Video;
    }
    UrlKind::Unknown
}

fn extract_after<'a>(url: &'a str, marker: &str) -> Option<&'a str> {
    let start = url.find(marker)? + marker.len();
    let rest = &url[start..];
    let end = rest.find(|c| c == '/' || c == '?' || c == '&' || c == '#').unwrap_or(rest.len());
    if end == 0 { None } else { Some(&rest[..end]) }
}

/// Lifecycle state of a download job.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Running,
    Done,
    Failed,
}

/// Internal message sent from the yt-dlp thread to the job.
enum Msg {
    Line(String),
    Progress(f32),
    Finished(bool),
}

/// A single yt-dlp invocation tracked by the downloader.
pub struct Job {
    pub url: String,
    /// Short human-readable path shown in the UI (e.g. `channels/handle/`).
    pub label: String,
    pub state: JobState,
    /// Download progress as a fraction in `[0.0, 1.0]`.
    pub progress: f32,
    /// Rolling log buffer — capped at 800 lines to avoid unbounded growth.
    pub log: Vec<String>,
    rx: Receiver<Msg>,
}

impl Job {
    fn drain(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Line(line) => {
                    self.log.push(line);
                    if self.log.len() > 800 {
                        let cut = self.log.len() - 800;
                        self.log.drain(0..cut);
                    }
                }
                Msg::Progress(p) => self.progress = p,
                Msg::Finished(ok) => {
                    self.state = if ok { JobState::Done } else { JobState::Failed };
                }
            }
        }
    }
}

/// Manages all active and recently completed yt-dlp download jobs.
pub struct Downloader {
    pub jobs: Vec<Job>,
    pub channels_root: PathBuf,
    /// Browser name passed to `--cookies-from-browser` (unused) — cookie file
    /// is currently always `cookies.txt`.
    pub browser: String,
}

impl Downloader {
    pub fn new(channels_root: PathBuf, browser: String) -> Self {
        Self { jobs: Vec::new(), channels_root, browser }
    }

    /// Spawn a yt-dlp process for `url` and track it as a new [`Job`].
    ///
    /// The output path template is derived from `kind` so that channels,
    /// playlists, and individual videos land in the right sub-directories.
    ///
    /// When `full_scan` is false (default / incremental mode) `--break-on-existing`
    /// is passed so yt-dlp stops as soon as it hits the first already-archived
    /// video — fast for routine channel checks.  When `full_scan` is true the
    /// flag is omitted so every video is checked individually against the
    /// download archive; slower, but correctly fills gaps in the history.
    pub fn start(&mut self, url: String, kind: &UrlKind, full_scan: bool) {
        let archive_path = self.channels_root.join("archive.txt");

        let (out_arg, label) = match kind {
            UrlKind::Channel { handle } => {
                let dir = self.channels_root.join(handle);
                let _ = std::fs::create_dir_all(&dir);
                (
                    format!("{}/%(title)s [%(id)s].%(ext)s", dir.display()),
                    format!("channels/{}/", handle),
                )
            }
            UrlKind::Playlist => (
                format!("{}/%(channel)s/%(playlist_title)s/%(title)s [%(id)s].%(ext)s", self.channels_root.display()),
                "channels/<channel>/<playlist>/".to_string(),
            ),
            UrlKind::Video | UrlKind::Unknown => (
                format!("{}/%(channel)s/%(title)s [%(id)s].%(ext)s", self.channels_root.display()),
                "channels/<channel>/".to_string(),
            ),
        };

        let mut cmd = Command::new("yt-dlp");
        cmd.arg("--newline")
            .arg("--no-color")
            .arg("--cookies")
            .arg("cookies.txt")
            .arg("--write-subs")
            .arg("--write-auto-subs")
            .arg("--write-thumbnail")
            .arg("--write-description")
            .arg("--write-info-json")
            // Don't write channel/playlist-level metafiles (avatar, info.json,
            // description). They land as "Title [CHANNEL_ID].ext" files that match
            // the per-video naming pattern and show up as phantom videos.
            .arg("--no-write-playlist-metafiles")
            .arg("--remux-video")
            .arg("mkv")
            .arg("--embed-metadata")
            .arg("--embed-info-json")
            .arg("--embed-chapters")
            .arg("--xattrs")
            .arg("--sponsorblock-mark")
            .arg("all")
            .arg("--extractor-args")
            .arg("youtube:player_client=web")
            .arg("--progress");
        if !full_scan {
            cmd.arg("--break-on-existing");
        }
        cmd.arg("--download-archive")
            .arg(archive_path.display().to_string())
            .arg("--impersonate")
            .arg("Chrome-146:Macos-26")
            .arg("-o")
            .arg(&out_arg)
            .arg(&url);

        self.spawn_job(cmd, url, label);
    }

    /// Re-fetch missing sidecar assets (thumbnail, info.json, description,
    /// subtitles) for a single video without re-downloading the video itself.
    ///
    /// `dir`/`stem` come from the existing file on disk so the fetched sidecars
    /// land with exactly the same filename stem and associate with the video.
    pub fn repair(&mut self, video_id: &str, dir: &std::path::Path, stem: &str) {
        let _ = std::fs::create_dir_all(dir);
        // Escape literal `%` so yt-dlp doesn't treat it as an output field.
        let safe_stem = stem.replace('%', "%%");
        let out_arg = format!("{}/{}.%(ext)s", dir.display(), safe_stem);
        let url = format!("https://www.youtube.com/watch?v={video_id}");
        let label = format!("repair {stem}");

        let mut cmd = Command::new("yt-dlp");
        cmd.arg("--newline")
            .arg("--no-color")
            .arg("--skip-download")
            .arg("--cookies")
            .arg("cookies.txt")
            .arg("--write-thumbnail")
            .arg("--write-info-json")
            .arg("--write-description")
            .arg("--write-subs")
            .arg("--write-auto-subs")
            .arg("--extractor-args")
            .arg("youtube:player_client=web")
            .arg("--impersonate")
            .arg("Chrome-146:Macos-26")
            .arg("-o")
            .arg(&out_arg)
            .arg(&url);

        self.spawn_job(cmd, url, label);
    }

    /// Spawn `cmd` on a background thread, streaming its output into a new [`Job`].
    fn spawn_job(&mut self, mut cmd: Command, url: String, label: String) {
        let (tx, rx) = channel();
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        thread::spawn(move || {
            let mut child = match cmd.spawn() {
                Ok(child) => child,
                Err(err) => {
                    let _ = tx.send(Msg::Line(format!("could not launch yt-dlp: {err}")));
                    let _ = tx.send(Msg::Finished(false));
                    return;
                }
            };

            if let Some(stderr) = child.stderr.take() {
                let tx = tx.clone();
                thread::spawn(move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        let _ = tx.send(Msg::Line(format!("[stderr] {line}")));
                    }
                });
            }

            if let Some(stdout) = child.stdout.take() {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    // Suppress the alarming "Aborting remaining downloads" line that
                    // yt-dlp emits for --break-on-existing; we replace it with a
                    // friendlier message when we detect exit code 101 below.
                    if line.trim() == "Aborting remaining downloads" {
                        continue;
                    }
                    if let Some(p) = parse_progress(&line) {
                        let _ = tx.send(Msg::Progress(p));
                    }
                    let _ = tx.send(Msg::Line(line));
                }
            }

            let ok = match child.wait() {
                Ok(status) if status.success() => true,
                // yt-dlp exits 101 when it stops early because of --break-on-existing
                // (it reached an already-archived video). That's the normal "nothing
                // new to download" outcome for a channel re-check, not a failure.
                Ok(status) => {
                    if status.code() == Some(101) {
                        let _ = tx.send(Msg::Line(
                            "(up to date — stopped at already-downloaded content)".to_string(),
                        ));
                        true
                    } else {
                        false
                    }
                }
                Err(_) => false,
            };
            let _ = tx.send(Msg::Finished(ok));
        });

        self.jobs.push(Job { url, label, state: JobState::Running, progress: 0.0, log: Vec::new(), rx });
    }

    /// Drain pending messages from all job threads into their log buffers.
    ///
    /// Call this regularly from the UI event loop to pick up progress updates.
    pub fn poll(&mut self) {
        for job in &mut self.jobs {
            job.drain();
        }
    }

    pub fn any_running(&self) -> bool {
        self.jobs.iter().any(|j| j.state == JobState::Running)
    }

    /// Remove all jobs that have finished (done or failed), keeping only running ones.
    pub fn clear_finished(&mut self) {
        self.jobs.retain(|j| j.state == JobState::Running);
    }

    /// Remove a single finished job by index.  Silently ignores the request
    /// if the job is still running.
    pub fn remove_job(&mut self, idx: usize) {
        if let Some(j) = self.jobs.get(idx) {
            if j.state != JobState::Running {
                self.jobs.remove(idx);
            }
        }
    }
}

/// Parse a yt-dlp `[download]  42.7% …` line into a `[0.0, 1.0]` fraction.
fn parse_progress(line: &str) -> Option<f32> {
    let rest = line.trim_start().strip_prefix("[download]")?.trim_start();
    let pct_end = rest.find('%')?;
    let value: f32 = rest[..pct_end].trim().parse().ok()?;
    Some((value / 100.0).clamp(0.0, 1.0))
}
