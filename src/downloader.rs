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
//! | `--impersonate <target>` | Browser TLS fingerprint per source platform (see [`crate::platform::Platform::impersonate_target`]) |
//! | `--break-on-existing` | Stop when the archive file records the video as already downloaded |
//! | `--download-archive archive.txt` | Record downloaded IDs to avoid re-downloading |

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::thread;

use crate::platform::{self, Platform, UrlInfo, UrlKind};
use crate::ytdlp_bin;

/// Video quality level passed as a `-f` format selector to yt-dlp.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum DownloadQuality {
    /// No `-f` flag — yt-dlp picks the best available streams (default).
    #[default]
    Best,
    Res1080,
    Res720,
    Res480,
    Res360,
}

impl DownloadQuality {
    pub fn format_spec(self) -> Option<&'static str> {
        match self {
            Self::Best => None,
            Self::Res1080 => Some("bestvideo[height<=1080]+bestaudio/best[height<=1080]"),
            Self::Res720  => Some("bestvideo[height<=720]+bestaudio/best[height<=720]"),
            Self::Res480  => Some("bestvideo[height<=480]+bestaudio/best[height<=480]"),
            Self::Res360  => Some("bestvideo[height<=360]+bestaudio/best[height<=360]"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Best    => "Best",
            Self::Res1080 => "1080p",
            Self::Res720  => "720p",
            Self::Res480  => "480p",
            Self::Res360  => "360p",
        }
    }

    pub fn all() -> &'static [DownloadQuality] {
        &[Self::Best, Self::Res1080, Self::Res720, Self::Res480, Self::Res360]
    }
}

/// Build a YouTube URL from a legacy library folder name. Used as a fallback
/// when a channel folder has no `.source-url` sidecar (i.e. it predates the
/// multi-platform changes).
///
/// Folder names that look like a channel ID (`UC` + 22 chars) use the
/// `/channel/` form; everything else is treated as a handle and gets `/@`.
/// This avoids the mismatch where info.json's canonical `channel_url` field
/// points to `/channel/UCxxx` and yt-dlp creates a second folder.
pub fn check_url_for_folder(folder_name: &str) -> String {
    if folder_name.starts_with("UC") && folder_name.len() == 24 {
        format!("https://www.youtube.com/channel/{folder_name}")
    } else {
        format!("https://www.youtube.com/@{folder_name}")
    }
}

/// Re-check URL for a [`crate::library::Channel`]. Prefers the stored
/// `.source-url` sidecar when present, falls back to the YouTube heuristic
/// for legacy folders.
pub fn recheck_url(ch: &crate::library::Channel) -> String {
    if let Some(url) = ch.source_url.as_deref() {
        return url.to_string();
    }
    // Legacy YouTube libraries: rebuild from folder name.
    check_url_for_folder(&ch.name)
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

/// Maximum lines retained in [`Job::log`] before old lines are evicted.
const JOB_LOG_CAP: usize = 800;

/// A single yt-dlp invocation tracked by the downloader.
pub struct Job {
    pub url: String,
    /// Short human-readable path shown in the UI (e.g. `channels/handle/`).
    pub label: String,
    pub state: JobState,
    /// Download progress as a fraction in `[0.0, 1.0]`.
    pub progress: f32,
    /// Rolling log buffer — capped at [`JOB_LOG_CAP`] lines via O(1) front-pop.
    pub log: VecDeque<String>,
    rx: Receiver<Msg>,
}

impl Job {
    fn drain(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Line(line) => {
                    self.log.push_back(line);
                    while self.log.len() > JOB_LOG_CAP {
                        self.log.pop_front();
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

/// A download waiting to start once a concurrency slot opens.
struct PendingJob {
    cmd: Command,
    url: String,
    label: String,
}

/// Manages all active, queued, and recently completed yt-dlp download jobs.
pub struct Downloader {
    pub jobs: Vec<Job>,
    pending: VecDeque<PendingJob>,
    pub channels_root: PathBuf,
    /// Browser name passed to `--cookies-from-browser` *when no cookies.txt
    /// exists in the working directory*. Set to `"none"` to skip the fallback
    /// entirely. Pasted/imported cookies.txt always takes precedence.
    pub browser: String,
    /// Maximum number of simultaneous yt-dlp processes. 0 = unlimited.
    pub max_concurrent: usize,
    /// If true, invoke the bundled yt-dlp under [`ytdlp_bin::bundled_dir`]
    /// instead of the system PATH yt-dlp.
    pub use_bundled_ytdlp: bool,
}

impl Downloader {
    pub fn new(channels_root: PathBuf, browser: String, max_concurrent: usize, use_bundled_ytdlp: bool) -> Self {
        Self {
            jobs: Vec::new(),
            pending: VecDeque::new(),
            channels_root,
            browser,
            max_concurrent,
            use_bundled_ytdlp,
        }
    }

    /// Append platform-specific extra flags. Currently only used to embed
    /// album art into the audio file on music-first platforms (Bandcamp,
    /// SoundCloud), where music players read embedded tags rather than
    /// scanning for sidecar JPEGs.
    fn apply_platform_extras(&self, platform: Platform, cmd: &mut Command) {
        if platform.is_audio_first() {
            cmd.arg("--embed-thumbnail");
        }
    }

    /// Append `--impersonate <target>` chosen per source platform. Both the
    /// bundled venv (which pip-installs `curl_cffi`) and a system yt-dlp
    /// with curl_cffi can satisfy this. Platforms that prefer no
    /// impersonation (e.g. Twitch's OAuth) return `None` from
    /// [`Platform::impersonate_target`] and the flag is omitted.
    fn apply_impersonation(&self, platform: Platform, cmd: &mut Command) {
        if let Some(target) = platform.impersonate_target() {
            cmd.arg("--impersonate").arg(target);
        }
    }

    /// Append the cookie-source flags. Prefers `cookies.txt` in the working
    /// directory (set up via the cookies UI), falling back to
    /// `--cookies-from-browser <browser>` when no cookies.txt exists and the
    /// user has chosen a browser (anything other than `"none"`).
    fn apply_cookie_flags(&self, cmd: &mut Command) {
        let cookies_txt = std::path::Path::new("cookies.txt");
        if cookies_txt.exists() {
            cmd.arg("--cookies").arg("cookies.txt");
        } else if !self.browser.is_empty() && self.browser != "none" {
            cmd.arg("--cookies-from-browser").arg(&self.browser);
        }
    }

    /// Append the retry and throttling flags applied to every yt-dlp invocation.
    ///
    /// YouTube occasionally resets connections mid-transfer; with default
    /// settings yt-dlp gives up after 10 quick retries. We bump the retry
    /// count and add a linear backoff so transient resets self-heal. The
    /// sleep flags throttle per-IP request rate slightly so we don't trip
    /// YouTube's rate limiter when many channels are being checked at once.
    fn apply_retry_flags(cmd: &mut Command) {
        cmd.arg("--retries").arg("30")
            .arg("--fragment-retries").arg("30")
            .arg("--retry-sleep").arg("linear=1:30:2")
            .arg("--sleep-requests").arg("1");
    }

    /// Build a fresh `Command` invoking the currently configured yt-dlp binary.
    ///
    /// In bundled mode, defensively re-applies the executable bit on every
    /// binary inside the bundled bin dir. Without this, downloads can fail
    /// with EACCES if a previous install left the file un-executable (e.g.
    /// because the chmod step of the install script never ran).
    fn ytdlp_cmd(&self) -> Command {
        let path = ytdlp_bin::ytdlp_invocation(self.use_bundled_ytdlp);
        if self.use_bundled_ytdlp {
            ytdlp_bin::ensure_bundled_executable();
        }
        Command::new(path)
    }

    /// Number of jobs waiting in the queue (not yet started).
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Labels and URLs of all queued (not yet started) jobs, in queue order.
    pub fn pending_snapshots(&self) -> Vec<(String, String)> {
        self.pending.iter().map(|p| (p.label.clone(), p.url.clone())).collect()
    }

    /// Promote pending jobs into running slots while capacity allows.
    fn promote_queued(&mut self) {
        while !self.pending.is_empty() {
            if self.max_concurrent > 0 {
                let running = self.jobs.iter().filter(|j| j.state == JobState::Running).count();
                if running >= self.max_concurrent { break; }
            }
            let p = self.pending.pop_front().unwrap();
            self.spawn_job(p.cmd, p.url, p.label);
        }
    }

    /// Push a command into the pending queue (or start immediately if a slot is free).
    fn enqueue(&mut self, cmd: Command, url: String, label: String) {
        self.pending.push_back(PendingJob { cmd, url, label });
        self.promote_queued();
    }

    /// Spawn a yt-dlp process for `url` and track it as a new [`Job`].
    ///
    /// Output path template is derived from `info.platform` + `info.kind` so
    /// each platform lands in its own sibling directory (`channels/` for
    /// YouTube, `tiktok/`, `twitch/`, etc. for others).
    ///
    /// For channel downloads we also drop a `.source-url` sidecar so future
    /// re-checks recover the original URL without folder-name guessing.
    ///
    /// When `full_scan` is false (default / incremental mode) `--break-on-existing`
    /// is passed so yt-dlp stops as soon as it hits the first already-archived
    /// video — fast for routine channel checks.  When `full_scan` is true the
    /// flag is omitted so every video is checked individually against the
    /// download archive; slower, but correctly fills gaps in the history.
    ///
    /// When `live` is true, the invocation is configured to record a live
    /// stream from the start: `--live-from-start --wait-for-video 30` is
    /// added, `--break-on-existing` is suppressed (each recording is unique),
    /// and the output filename gains a UTC timestamp suffix so re-recordings
    /// of the same channel don't collide.
    pub fn start(&mut self, url: String, info: &UrlInfo, full_scan: bool, quality: DownloadQuality, live: bool) {
        let platform_dir = platform::platform_root(&self.channels_root, info.platform);
        // Per-platform download archive keeps cross-platform IDs from colliding
        // (TikTok IDs are numeric, YouTube IDs are 11-char base64, etc.).
        let _ = std::fs::create_dir_all(&platform_dir);
        let archive_path = platform_dir.join("archive.txt");
        let platform_label = info.platform.dir_name();

        // Live recordings get a UTC timestamp suffix in the filename so a
        // re-recording of the same stream doesn't overwrite the prior one.
        // VOD downloads rely on yt-dlp's stable `%(id)s` for uniqueness.
        let live_suffix = if live {
            format!(" [{}]", format_compact_utc(now_unix()))
        } else {
            String::new()
        };
        let live_label = if live { " 🔴 LIVE" } else { "" };

        let (out_arg, label) = match &info.kind {
            UrlKind::Channel { handle } => {
                let dir = platform_dir.join(handle);
                let _ = std::fs::create_dir_all(&dir);
                // Remember the originating URL so re-checks don't have to
                // guess from the folder name.
                platform::write_source_url(&dir, &url);
                // Bandcamp at the bare artist URL is a whole discography.
                // Organize each track into its album subfolder so the
                // resulting tree mirrors how Bandcamp itself presents the
                // catalog. Other platforms keep their flat per-creator layout.
                let template = if info.platform == Platform::Bandcamp {
                    format!(
                        "{}/%(album|Unknown)s/%(title)s [%(id)s]{live_suffix}.%(ext)s",
                        dir.display()
                    )
                } else {
                    format!("{}/%(title)s [%(id)s]{live_suffix}.%(ext)s", dir.display())
                };
                (template, format!("{}/{}/{}", platform_label, handle, live_label))
            }
            UrlKind::Playlist => (
                format!(
                    "{}/%(uploader,channel,creator|Unknown)s/%(playlist_title)s/%(title)s [%(id)s]{live_suffix}.%(ext)s",
                    platform_dir.display()
                ),
                format!("{}/<creator>/<playlist>/{}", platform_label, live_label),
            ),
            UrlKind::Video | UrlKind::Unknown => (
                format!(
                    "{}/%(uploader,channel,creator|Unknown)s/%(title)s [%(id)s]{live_suffix}.%(ext)s",
                    platform_dir.display()
                ),
                format!("{}/<creator>/{}", platform_label, live_label),
            ),
        };

        let mut cmd = self.ytdlp_cmd();
        cmd.arg("--newline").arg("--no-color");
        self.apply_cookie_flags(&mut cmd);
        cmd.arg("--write-subs")
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
        if let Some(fmt) = quality.format_spec() {
            cmd.arg("-f").arg(fmt);
        }
        if live {
            // Record the broadcast from the start instead of joining live.
            // `--wait-for-video` polls the URL every 30 s until a stream
            // is actually live, so scheduling a recording before the
            // stream begins works naturally.
            cmd.arg("--live-from-start").arg("--wait-for-video").arg("30");
        } else if !full_scan {
            // Live recordings should never short-circuit on existing archive
            // entries — every recording is its own file. Only honor
            // `--break-on-existing` for VOD/channel-check downloads.
            cmd.arg("--break-on-existing");
        }
        cmd.arg("--download-archive")
            .arg(archive_path.display().to_string());
        self.apply_impersonation(info.platform, &mut cmd);
        self.apply_platform_extras(info.platform, &mut cmd);
        cmd.arg("-o").arg(&out_arg).arg(&url);
        Self::apply_retry_flags(&mut cmd);

        self.enqueue(cmd, url, label);
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

        let mut cmd = self.ytdlp_cmd();
        cmd.arg("--newline").arg("--no-color").arg("--skip-download");
        self.apply_cookie_flags(&mut cmd);
        cmd.arg("--write-thumbnail")
            .arg("--write-info-json")
            .arg("--write-description")
            .arg("--write-subs")
            .arg("--write-auto-subs")
            .arg("--extractor-args")
            .arg("youtube:player_client=web");
        // `repair()` rebuilds a YouTube watch URL from a stored video ID, so
        // the source platform is always YouTube here regardless of where the
        // original video lives on disk.
        self.apply_impersonation(Platform::YouTube, &mut cmd);
        cmd.arg("-o").arg(&out_arg).arg(&url);
        Self::apply_retry_flags(&mut cmd);

        self.enqueue(cmd, url, label);
    }

    /// Path to the music download directory (sibling of `channels_root`).
    pub fn music_root(&self) -> PathBuf {
        self.channels_root.with_file_name("music")
    }

    /// Download `url` as audio-only, storing tracks in `music/<artist>/`.
    pub fn start_music(&mut self, url: String) {
        let music_root = self.music_root();
        let _ = std::fs::create_dir_all(&music_root);
        let archive_path = self.channels_root.join("archive.txt");
        let out_arg = format!(
            "{}/%(artist,channel|Unknown)s/%(title)s [%(id)s].%(ext)s",
            music_root.display()
        );
        let label = "music".to_string();

        let mut cmd = self.ytdlp_cmd();
        cmd.arg("--newline").arg("--no-color");
        self.apply_cookie_flags(&mut cmd);
        cmd.arg("--extract-audio")
            .arg("--audio-format")
            .arg("best")
            .arg("--audio-quality")
            .arg("0")
            .arg("--write-thumbnail")
            .arg("--write-info-json")
            .arg("--embed-metadata")
            .arg("--xattrs")
            .arg("--extractor-args")
            .arg("youtube:player_client=web");
        // Music downloads can come from any audio-first platform — classify
        // the URL once so SoundCloud/Bandcamp pulls get their appropriate
        // (typically no-op) impersonation profile.
        let platform = platform::classify_url(&url).platform;
        self.apply_impersonation(platform, &mut cmd);
        cmd.arg("--progress")
            .arg("--download-archive")
            .arg(archive_path.display().to_string())
            .arg("-o")
            .arg(&out_arg)
            .arg(&url);
        Self::apply_retry_flags(&mut cmd);

        self.enqueue(cmd, url, label);
    }

    /// Enqueue a job that downloads (or updates) the bundled yt-dlp + deno
    /// binaries into [`ytdlp_bin::bundled_dir`]. Streams the curl/unzip output
    /// into a normal [`Job`] entry so the user sees progress in the UI.
    pub fn start_ytdlp_update(&mut self) {
        let cmd = ytdlp_bin::install_command();
        let url = "https://github.com/yt-dlp/yt-dlp/releases/latest".to_string();
        let label = "update bundled yt-dlp + deno".to_string();
        self.enqueue(cmd, url, label);
    }

    /// Spawn `cmd` on a background thread, streaming its output into a new [`Job`].
    fn spawn_job(&mut self, mut cmd: Command, url: String, label: String) {
        let (tx, rx) = channel();
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Prepend the bundled bin dir to PATH so yt-dlp can locate the bundled
        // `deno` for JavaScript signature deciphering. Harmless when the dir
        // doesn't exist or bundled mode is disabled.
        let bundled_dir = ytdlp_bin::bundled_dir();
        if bundled_dir.exists() {
            let sep = if cfg!(windows) { ";" } else { ":" };
            let new_path = match std::env::var_os("PATH") {
                Some(existing) => format!("{}{}{}", bundled_dir.display(), sep, existing.to_string_lossy()),
                None => bundled_dir.display().to_string(),
            };
            cmd.env("PATH", new_path);
        }

        // Absolute path that we want to redact out of any log line — yt-dlp
        // sometimes echoes the cookie path in errors and that leaks the
        // user's home directory into the UI / API responses.
        let cookies_abs = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("cookies.txt")
            .display()
            .to_string();

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
                let cookies_abs = cookies_abs.clone();
                thread::spawn(move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        let line = redact_sensitive(&line, &cookies_abs);
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
                    let line = redact_sensitive(&line, &cookies_abs);
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

        self.jobs.push(Job { url, label, state: JobState::Running, progress: 0.0, log: VecDeque::new(), rx });
    }

    /// Drain pending messages from all job threads and promote queued jobs.
    ///
    /// Call this regularly from the UI event loop to pick up progress updates.
    pub fn poll(&mut self) {
        for job in &mut self.jobs {
            job.drain();
        }
        // Re-check after draining: finished jobs free slots for queued ones.
        self.promote_queued();
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

/// Current UNIX timestamp in seconds. Used to disambiguate live-recording
/// filenames at job-start time.
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Format a UNIX timestamp as `YYYYMMDD-HHMMSS` (UTC) for embedding in
/// filenames. No `chrono` dep — short manual calendar walk; good enough
/// for human-readable filename suffixes.
fn format_compact_utc(unix: u64) -> String {
    let day = unix / 86_400;
    let day_secs = unix % 86_400;
    let hour = day_secs / 3600;
    let minute = (day_secs % 3600) / 60;
    let second = day_secs % 60;

    let mut year = 1970u32;
    let mut remaining_days = day;
    loop {
        let leap = is_leap(year);
        let yd = if leap { 366 } else { 365 };
        if remaining_days < yd as u64 { break; }
        remaining_days -= yd as u64;
        year += 1;
    }
    let months: [u8; 12] = if is_leap(year) {
        [31,29,31,30,31,30,31,31,30,31,30,31]
    } else {
        [31,28,31,30,31,30,31,31,30,31,30,31]
    };
    let mut month = 0usize;
    while month < 12 && remaining_days >= months[month] as u64 {
        remaining_days -= months[month] as u64;
        month += 1;
    }
    let day_of_month = remaining_days as u32 + 1;
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        year, month as u32 + 1, day_of_month, hour, minute, second
    )
}

fn is_leap(y: u32) -> bool { (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 }

/// Strip the absolute on-disk path to `cookies.txt` from a log line, leaving
/// the bare filename. yt-dlp occasionally echoes the full path in error
/// messages ("Unable to load cookies from /home/user/.../cookies.txt"); the
/// user's home directory is not something we want to expose in the UI or
/// `/api/progress` responses.
fn redact_sensitive(line: &str, cookies_abs: &str) -> String {
    if cookies_abs.is_empty() || !line.contains(cookies_abs) {
        return line.to_string();
    }
    line.replace(cookies_abs, "cookies.txt")
}

/// Parse a yt-dlp `[download]  42.7% …` line into a `[0.0, 1.0]` fraction.
fn parse_progress(line: &str) -> Option<f32> {
    let rest = line.trim_start().strip_prefix("[download]")?.trim_start();
    let pct_end = rest.find('%')?;
    let value: f32 = rest[..pct_end].trim().parse().ok()?;
    Some((value / 100.0).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_progress_typical() {
        let p = parse_progress("[download]  42.7% of 100MiB at 5MiB/s ETA 00:10").unwrap();
        assert!((p - 0.427).abs() < 1e-4);
    }

    #[test]
    fn parse_progress_clamps_to_one() {
        let p = parse_progress("[download]  150% of garbage").unwrap();
        assert_eq!(p, 1.0);
    }

    #[test]
    fn parse_progress_rejects_non_download_lines() {
        assert!(parse_progress("[info] Writing thumbnail").is_none());
        assert!(parse_progress("").is_none());
    }

    #[test]
    fn check_url_for_folder_picks_channel_form_for_ids() {
        let url = check_url_for_folder("UC1234567890123456789012");
        assert_eq!(url, "https://www.youtube.com/channel/UC1234567890123456789012");
    }

    #[test]
    fn check_url_for_folder_picks_handle_form_otherwise() {
        let url = check_url_for_folder("LinusTechTips");
        assert_eq!(url, "https://www.youtube.com/@LinusTechTips");
    }

    #[test]
    fn redact_sensitive_strips_cookie_path() {
        let abs = "/home/luna/.config/yt-offline/cookies.txt";
        let input = format!("Unable to load cookies from {abs}");
        let out = redact_sensitive(&input, abs);
        assert_eq!(out, "Unable to load cookies from cookies.txt");
    }

    #[test]
    fn redact_sensitive_pass_through_when_not_present() {
        let abs = "/home/luna/cookies.txt";
        let input = "[download]  47.2% of 100MiB";
        assert_eq!(redact_sensitive(input, abs), input);
    }
    // URL classification tests live in `platform` now — see its tests module.
}
