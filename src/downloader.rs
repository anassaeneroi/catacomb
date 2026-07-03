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
//! | `--write-subs` (+ optionally `--write-auto-subs` / `--sub-langs` / `--convert-subs` / `--embed-subs`) | Subtitles, per the `[subtitles]` config + per-channel overrides |
//! | `--write-thumbnail` | Download channel/video thumbnails |
//! | `--write-description` | Save video description as a sidecar `.description` file |
//! | `--write-info-json` | Save full metadata as a `.info.json` sidecar |
//! | `--remux-video mkv` | Re-container to MKV (no re-encode) |
//! | `--embed-metadata --embed-info-json --embed-chapters` | Embed rich metadata into the MKV |
//! | `--xattrs` | Store metadata in filesystem extended attributes |
//! | `--sponsorblock-mark/-remove all` | SponsorBlock handling, per `sponsorblock_mode` (off/mark/remove; see [`Self::apply_sponsorblock`]) |
//! | `--write-comments` | Fetch the comment tree, per `fetch_comments` global + per-channel override (see [`Self::apply_comments`]) |
//! | `--impersonate <target>` | Browser TLS fingerprint per source platform (see [`crate::platform::Platform::impersonate_target`]) |
//! | `--break-on-existing` | Stop when the archive file records the video as already downloaded |
//! | `--download-archive archive.txt` | Record downloaded IDs to avoid re-downloading |

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::thread;

use crate::download_options::DownloadOptions;
use crate::platform::{self, Platform, UrlInfo, UrlKind};
use crate::ytdlp_bin;

/// Video quality level passed as a `-f` format selector to yt-dlp.
#[derive(Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    /// Best-effort classification of the failure, populated when `state`
    /// transitions to `Failed`. `None` while running or on success. The UI
    /// surfaces the class + a one-line suggested fix from
    /// [`crate::error_class`].
    pub failure_class: Option<crate::error_class::ErrorClass>,
    /// Inputs needed to re-run this job, captured at start time. Lets the
    /// downloader rebuild the command for an automatic retry after a
    /// transient (rate-limit / network) failure without the caller
    /// re-issuing it. `None` for jobs that aren't retryable (e.g. the
    /// yt-dlp self-update job).
    retry_spec: Option<RetrySpec>,
    /// How many automatic retries this job has already consumed. Capped at
    /// [`MAX_AUTO_RETRIES`] so a permanently-blocked video doesn't loop.
    retry_count: u8,
    /// Set once we've evaluated this job's failure for auto-retry, so the
    /// per-poll scan doesn't schedule the same failure repeatedly.
    retry_handled: bool,
    /// Post-download conversion to run on this job's finished files.
    /// `None` for jobs that don't convert (most jobs). Consumed once when
    /// the job transitions to Done.
    convert_on_finish: Option<ConvertSpec>,
    /// Set once we've enqueued the convert pass for this finished job, so
    /// the per-poll scan doesn't enqueue it twice.
    convert_handled: bool,
    /// OS pid of the child process, captured at spawn. Lets the hang
    /// watchdog SIGKILL a stalled process (Unix). `None` for synthetic
    /// jobs (preflight failures) that have no process.
    child_pid: Option<u32>,
    /// Last time this job produced any output/progress. The watchdog kills
    /// a Running job whose `last_activity` is older than [`HANG_TIMEOUT`].
    last_activity: std::time::Instant,
    /// True when the watchdog killed this job (vs. a genuine yt-dlp exit).
    /// On the resulting failure we force a retryable class so auto-retry
    /// re-queues it — a hang is transient, worth one more try.
    watchdog_killed: bool,
    /// True when the user explicitly cancelled this job (vs. a genuine
    /// failure). Suppresses auto-retry and is surfaced as a distinct
    /// "cancelled" state in the UI rather than a misleading error class.
    pub cancelled: bool,
    rx: Receiver<Msg>,
}

/// Captured inputs for rebuilding a download command on auto-retry.
#[derive(Clone)]
struct RetrySpec {
    url: String,
    info: UrlInfo,
    full_scan: bool,
    quality: DownloadQuality,
    live: bool,
    opts: Option<DownloadOptions>,
}

/// Resolved post-download conversion settings, attached to a download Job
/// so the downloader knows what ffmpeg pass (if any) to run on each
/// finished file. `mode` is one of remux-mp4 / h264-mp4 / audio; the
/// other fields parameterise it.
#[derive(Clone)]
pub struct ConvertSpec {
    pub mode: String,
    pub crf: u8,
    pub preset: String,
    pub audio_format: String,
    pub keep_original: bool,
}

impl ConvertSpec {
    /// Build a [`ConvertSpec`] from the global `[convert]` config merged
    /// with a per-channel `convert_mode` override. Returns `None` when the
    /// effective mode is off/empty (no transcode pass).
    fn resolve(
        global: &crate::config::ConvertSection,
        opts: Option<&DownloadOptions>,
    ) -> Option<ConvertSpec> {
        let mode = opts
            .and_then(|o| o.convert_mode.as_deref())
            .filter(|s| !s.is_empty())
            .unwrap_or(global.mode.as_str())
            .trim()
            .to_string();
        if mode.is_empty() || mode == "off" {
            return None;
        }
        Some(ConvertSpec {
            mode,
            crf: if global.crf == 0 { 23 } else { global.crf },
            preset: if global.preset.is_empty() { "medium".into() } else { global.preset.clone() },
            audio_format: if global.audio_format.is_empty() { "mp3".into() } else { global.audio_format.clone() },
            keep_original: global.keep_original,
        })
    }
}

/// Max automatic re-queues per job for transient failures. One retry
/// clears most momentary captchas/resets; more than that and it's a real
/// block the user needs to act on (refresh cookies, wait longer).
const MAX_AUTO_RETRIES: u8 = 2;

/// Cooldown before an auto-retry fires. Long enough to let a momentary
/// rate-limit window pass; short enough that the user isn't left waiting.
const AUTO_RETRY_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(90);

/// A Running job that produces no output/progress for this long is
/// considered hung and gets SIGKILLed by the watchdog. Chosen well above
/// any legitimate quiet gap: yt-dlp prints a progress line per chunk
/// while downloading, polls every 30 s under `--wait-for-video`, and our
/// longest configured sleep (retry-sleep cap / adaptive throttle) is
/// ~30 s — so 5 minutes of total silence means a real stall (a stuck TLS
/// handshake or unresponsive server), not slow-but-alive progress.
const HANG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

impl Job {
    /// Whether this job carries the inputs needed for a manual retry (download
    /// jobs do; live recordings, self-update, and synthetic preflight failures
    /// don't). Drives the UI's "Retry" button.
    pub fn has_retry_spec(&self) -> bool {
        self.retry_spec.is_some()
    }

    fn drain(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Line(line) => {
                    self.last_activity = std::time::Instant::now();
                    self.log.push_back(line);
                    while self.log.len() > JOB_LOG_CAP {
                        self.log.pop_front();
                    }
                }
                Msg::Progress(p) => {
                    self.last_activity = std::time::Instant::now();
                    self.progress = p;
                }
                Msg::Finished(ok) => {
                    self.state = if ok { JobState::Done } else { JobState::Failed };
                    // Classify only on the failure transition so the
                    // classifier doesn't re-run for every poll() call on a
                    // long-finished job. The log is already in `self.log`
                    // by this point since we drained Line messages above.
                    if !ok && self.failure_class.is_none() {
                        self.failure_class = Some(if self.watchdog_killed {
                            // A watchdog kill leaves no telltale error line, so
                            // classify() would return Other (non-retryable).
                            // Force NetworkError: a hang is transient and the
                            // auto-retry path should give it another go.
                            crate::error_class::ErrorClass::NetworkError
                        } else {
                            crate::error_class::classify(self.log.iter().map(|s| s.as_str()))
                        });
                    }
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
    /// Retry spec captured at enqueue time (None for non-retryable jobs).
    retry_spec: Option<RetrySpec>,
    /// Attempt number this command represents (0 = first try).
    retry_count: u8,
    /// Post-download convert spec for this job (None = no conversion).
    convert_spec: Option<ConvertSpec>,
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
    /// If true and the bundled yt-dlp is in use, spawn the bgutil-pot
    /// HTTP server on first download and pass its extractor-args to
    /// every yt-dlp invocation. See [`crate::pot_provider`].
    pub use_pot_provider: bool,
    /// Running bgutil-pot server child. Lazily spawned in [`Self::start`]
    /// on the first job after the flag turns on; killed in [`Drop`].
    pot_server: Option<std::process::Child>,
    /// Global subtitle defaults from `[subtitles]` config. Per-channel
    /// [`DownloadOptions`] override individual fields. Set by the app /
    /// web layer at construction and on settings save.
    pub subtitle_defaults: crate::config::SubtitlesSection,
    /// Global `backup.youtube_player_clients` (comma-separated). Empty =
    /// yt-dlp defaults. Per-channel options can override. Set at
    /// construction + on settings save.
    pub youtube_player_clients: String,
    /// Global `backup.sponsorblock_mode` ("off" / "mark" / "remove").
    /// Per-channel options can override. Set at construction + on save.
    pub sponsorblock_mode: String,
    /// Global `backup.fetch_comments`. When true, downloads pass
    /// `--write-comments`. Per-channel options can override. Set at
    /// construction + on settings save.
    pub fetch_comments: bool,
    /// Global `backup.dedup_enabled`. When false, the perceptual "similar
    /// content" scan is hard-disabled in both UIs. Set at construction + save.
    pub dedup_enabled: bool,
    /// Global `[convert]` config. Drives the post-download ffmpeg pass.
    /// Per-channel options override the mode. Set at construction + save.
    pub convert_defaults: crate::config::ConvertSection,
    /// Scheduled auto-retries: `(fire_at, spec, attempt_number)`. Populated
    /// when a job fails with a retryable class; drained in [`Self::poll`]
    /// once `fire_at` passes, re-issuing the download. Kept separate from
    /// `pending` so the cooldown is honored without blocking a slot.
    retry_queue: Vec<(std::time::Instant, RetrySpec, u8)>,
    /// True while the most recent rate-limit hit's adaptive backoff is in
    /// effect. Set when a job fails rate-limited; makes subsequent jobs in
    /// the batch sleep longer. Cleared once downloads go quiet.
    pub rate_limited_backoff: bool,
    /// Stashed by [`Self::start`] just before it enqueues, consumed by the
    /// next [`Self::spawn_job`] so the resulting [`Job`] carries the spec
    /// for auto-retry. Avoids threading `Option<RetrySpec>` through the
    /// four non-retryable enqueue paths (repair / music / updates).
    pending_retry_spec: Option<RetrySpec>,
    /// Attempt number for the job [`Self::start`] is about to enqueue.
    /// 0 for a user-initiated download; >0 when [`Self::start_retry`] is
    /// re-issuing a failed one. Consumed by `enqueue`.
    retry_attempt_override: u8,
    /// Stashed by [`Self::start`], consumed by the next `enqueue` so the
    /// download job carries its convert spec. Like `pending_retry_spec`.
    pending_convert_spec: Option<ConvertSpec>,
}

impl Downloader {
    pub fn new(
        channels_root: PathBuf,
        browser: String,
        max_concurrent: usize,
        use_bundled_ytdlp: bool,
        use_pot_provider: bool,
    ) -> Self {
        Self {
            jobs: Vec::new(),
            pending: VecDeque::new(),
            channels_root,
            browser,
            max_concurrent,
            use_bundled_ytdlp,
            use_pot_provider,
            pot_server: None,
            subtitle_defaults: crate::config::SubtitlesSection::default(),
            youtube_player_clients: String::new(),
            sponsorblock_mode: "mark".to_string(),
            fetch_comments: false,
            dedup_enabled: true,
            convert_defaults: crate::config::ConvertSection::default(),
            retry_queue: Vec::new(),
            rate_limited_backoff: false,
            pending_retry_spec: None,
            retry_attempt_override: 0,
            pending_convert_spec: None,
        }
    }

    /// Resolve global `[subtitles]` config + per-channel overrides into
    /// the yt-dlp subtitle flag set, appending to `cmd`.
    ///
    /// Resolution: a per-channel `Some` wins over the global default for
    /// each field. When subtitles are disabled (globally or per-channel),
    /// nothing is emitted — yt-dlp then writes no subs.
    /// Append `--extractor-args youtube:player_client=…` when a client
    /// list is configured (per-channel override or global default). Empty
    /// = omit the flag entirely so yt-dlp uses its own default client set
    /// (the recommended baseline). Only meaningful for YouTube; the
    /// youtube: namespace is ignored by other extractors so it's harmless
    /// to always pass.
    fn apply_player_client(&self, cmd: &mut Command, opts: Option<&DownloadOptions>) {
        let clients = opts
            .and_then(|o| o.youtube_player_clients.as_deref())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(self.youtube_player_clients.as_str())
            .trim();
        if clients.is_empty() { return; }
        cmd.arg("--extractor-args")
            .arg(format!("youtube:player_client={clients}"));
    }

    /// Apply SponsorBlock flags from the resolved mode (per-channel override
    /// or global default). "mark" chapter-marks segments, "remove" cuts them,
    /// anything else (incl. "off") omits the flags entirely.
    fn apply_sponsorblock(&self, cmd: &mut Command, opts: Option<&DownloadOptions>) {
        let mode = opts
            .and_then(|o| o.sponsorblock_mode.as_deref())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(self.sponsorblock_mode.as_str())
            .trim();
        match mode {
            "mark" => { cmd.arg("--sponsorblock-mark").arg("all"); }
            "remove" => { cmd.arg("--sponsorblock-remove").arg("all"); }
            _ => {} // "off" or unknown → no SponsorBlock processing
        }
    }

    /// Add `--write-comments` when comment fetching is enabled, merging the
    /// per-channel override (`Some(true)`/`Some(false)`) with the global
    /// `fetch_comments` default (used when the override is `None`).
    fn apply_comments(&self, cmd: &mut Command, opts: Option<&DownloadOptions>) {
        let enabled = opts
            .and_then(|o| o.fetch_comments)
            .unwrap_or(self.fetch_comments);
        if enabled {
            cmd.arg("--write-comments");
        }
    }

    fn apply_subtitle_flags(&self, cmd: &mut Command, opts: Option<&DownloadOptions>) {
        let g = &self.subtitle_defaults;
        // Per-channel override-or-global for each knob.
        let enabled = opts.and_then(|o| o.subtitles_enabled).unwrap_or(g.enabled);
        if !enabled { return; }
        let auto = opts.and_then(|o| o.subtitles_auto).unwrap_or(g.auto_generated);
        let embed = opts.and_then(|o| o.subtitles_embed).unwrap_or(g.embed);
        // Format: per-channel Some(non-empty) wins; else global.
        let format = opts
            .and_then(|o| o.subtitle_format.as_deref())
            .filter(|s| !s.is_empty())
            .unwrap_or(g.format.as_str());
        // Langs: per-channel list wins; else global comma string.
        let langs: String = match opts {
            Some(o) if !o.subtitle_langs.is_empty() => o.subtitle_langs.join(","),
            _ => g.langs.clone(),
        };

        cmd.arg("--write-subs");
        if auto {
            cmd.arg("--write-auto-subs");
        }
        if !langs.is_empty() {
            cmd.arg("--sub-langs").arg(langs);
        }
        if !format.is_empty() {
            cmd.arg("--convert-subs").arg(format);
        }
        if embed {
            cmd.arg("--embed-subs");
        }
    }

    /// Lazy-spawn the POT server on first use. We don't spin it up on
    /// app start because most users won't have it installed; doing the
    /// check + spawn on first download means there's no per-launch
    /// penalty when the feature is off, and an obvious place to surface
    /// "did you install the binary?" errors when it's on.
    fn ensure_pot_server(&mut self) {
        if !self.use_pot_provider { return; }
        if !self.use_bundled_ytdlp { return; } // plugin is in the bundled venv
        if self.pot_server.is_some() { return; } // already running
        if !crate::pot_provider::installed() { return; } // not installed, skip silently
        match crate::pot_provider::spawn_server() {
            Ok(child) => { self.pot_server = Some(child); }
            Err(_) => { /* failure surfaces as missing POT → yt-dlp warns */ }
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
    /// count and add a linear backoff so transient resets self-heal.
    ///
    /// The sleep flags throttle request rate so we don't trip YouTube's
    /// bot-detection / captcha wall mid-batch (it tends to fire after
    /// ~30 rapid requests):
    /// - `--sleep-requests`: pause between metadata/API requests.
    /// - `--sleep-interval` / `--max-sleep-interval`: a random pause
    ///   *between videos*. The jitter is what matters — a fixed cadence
    ///   looks robotic; a random one looks human and is far less likely
    ///   to trip the captcha on a long channel scan.
    ///
    /// Adaptive backoff: after a rate-limit hit (`rate_limited_backoff`),
    /// the sleeps roughly triple for the rest of the batch so we ease off
    /// instead of hammering an already-suspicious endpoint.
    fn apply_retry_flags(&self, cmd: &mut Command) {
        let (req, lo, hi) = if self.rate_limited_backoff {
            ("3", "8", "20")
        } else {
            ("1", "2", "6")
        };
        cmd.arg("--retries").arg("30")
            .arg("--fragment-retries").arg("30")
            .arg("--retry-sleep").arg("linear=1:30:2")
            .arg("--sleep-requests").arg(req)
            .arg("--sleep-interval").arg(lo)
            .arg("--max-sleep-interval").arg(hi);
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
            self.spawn_job(p.cmd, p.url, p.label, p.retry_spec, p.retry_count, p.convert_spec);
        }
    }

    /// Push a command into the pending queue (or start immediately if a slot is free).
    ///
    /// Consumes `self.pending_retry_spec` + `self.pending_convert_spec`
    /// (set by [`Self::start`]) so the resulting job carries them, and
    /// `retry_attempt_override` for the attempt number (0 on a first run,
    /// >0 from a retry). Other callers (repair, music, updates) leave the
    /// specs `None`, making those jobs non-retryable + non-converting.
    fn enqueue(&mut self, cmd: Command, url: String, label: String) {
        let retry_spec = self.pending_retry_spec.take();
        let retry_count = self.retry_attempt_override;
        let convert_spec = self.pending_convert_spec.take();
        self.pending.push_back(PendingJob { cmd, url, label, retry_spec, retry_count, convert_spec });
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
    ///
    /// `channel_options` carries per-channel overrides (rate limit, filters,
    /// extra args, …) and is applied after the standard flag set so it can
    /// win. Pass `None` when the caller doesn't know which channel the URL
    /// belongs to (e.g. an arbitrary URL pasted into the download dialog).
    /// The channel-options `quality` field overrides the `quality` parameter
    /// only when the caller explicitly opts in by passing it through.
    pub fn start(
        &mut self,
        url: String,
        info: &UrlInfo,
        full_scan: bool,
        quality: DownloadQuality,
        live: bool,
        channel_options: Option<&DownloadOptions>,
    ) {
        // Make sure the POT server is up before we hand the URL to
        // yt-dlp — the Python plugin checks the HTTP endpoint at
        // extractor-time, so a too-late start means the first request
        // misses POT and YouTube hands back empty formats.
        self.ensure_pot_server();

        // Capture the inputs so spawn_job can attach a RetrySpec for
        // automatic retry on a transient (rate-limit / network) failure.
        // Live recordings aren't retried — re-running would start a fresh
        // recording, not resume.
        self.pending_retry_spec = if live {
            None
        } else {
            Some(RetrySpec {
                url: url.clone(),
                info: info.clone(),
                full_scan,
                quality,
                live,
                opts: channel_options.cloned(),
            })
        };

        let platform_dir = platform::platform_root(&self.channels_root, info.platform);
        // Per-platform download archive keeps cross-platform IDs from colliding
        // (TikTok IDs are numeric, YouTube IDs are 11-char base64, etc.).
        let _ = std::fs::create_dir_all(&platform_dir);

        // Disk-full preflight. Refuse to spawn yt-dlp when the target
        // filesystem has less than the floor of free space — that's a
        // near-certain in-progress ENOSPC, which leaves a partial file
        // and a confusing failure. We surface it as a synthetic Failed
        // job classified as DiskFull so it appears in the Downloads
        // panel alongside other classified errors instead of vanishing.
        //
        // statvfs returning None (non-Unix host, missing path) skips the
        // check — better to let yt-dlp run and produce a real error
        // than refuse on missing data.
        if let Some(free) = crate::disk_space::available_bytes(&self.channels_root) {
            if free < crate::disk_space::FREE_SPACE_FLOOR_BYTES {
                self.push_synthetic_failure(
                    url,
                    platform_dir.display().to_string(),
                    crate::error_class::ErrorClass::DiskFull,
                    format!(
                        "only {} free on {} (need at least {})",
                        crate::disk_space::fmt_bytes(free),
                        self.channels_root.display(),
                        crate::disk_space::fmt_bytes(crate::disk_space::FREE_SPACE_FLOOR_BYTES),
                    ),
                );
                return;
            }
        }

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
        cmd.arg("--write-thumbnail")
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
        // Subtitle flags: global [subtitles] config merged with per-channel
        // overrides. Done here (not in opts.apply) so it works even when a
        // channel has no options row.
        self.apply_subtitle_flags(&mut cmd, channel_options);
        // YouTube player-client selection (global default + per-channel
        // override). Lets the user route around a captcha-walled client.
        self.apply_player_client(&mut cmd, channel_options);
        // SponsorBlock: global default + per-channel override (off/mark/remove).
        self.apply_sponsorblock(&mut cmd, channel_options);
        // Comment fetching: global backup.fetch_comments + per-channel override.
        self.apply_comments(&mut cmd, channel_options);
        // Post-download conversion: resolve global [convert] + per-channel
        // override. When active, ask yt-dlp to print each finished file's
        // final path (after_move) so we can enqueue an ffmpeg pass on it.
        let convert_spec = ConvertSpec::resolve(&self.convert_defaults, channel_options);
        if convert_spec.is_some() {
            // The CONVERT_PATH: prefix lets spawn_job's stdout reader
            // recognise these lines without confusing them for progress.
            cmd.arg("--print").arg("after_move:CONVERT_PATH:%(filepath)s");
        }
        self.pending_convert_spec = convert_spec;
        // Per-channel option overrides win when present — they're applied
        // last so a `--limit-rate` / `--match-filter` / passthrough arg from
        // the channel settings takes priority over the global defaults.
        if let Some(opts) = channel_options {
            opts.apply(&mut cmd);
        }
        cmd.arg("-o").arg(&out_arg).arg(&url);
        self.apply_retry_flags(&mut cmd);

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
            .arg("--write-description");
        // Repair has no channel-options context, so subtitles use the
        // global [subtitles] defaults.
        self.apply_subtitle_flags(&mut cmd, None);
        // `repair()` rebuilds a YouTube watch URL from a stored video ID, so
        // the source platform is always YouTube here regardless of where the
        // original video lives on disk.
        self.apply_impersonation(Platform::YouTube, &mut cmd);
        cmd.arg("-o").arg(&out_arg).arg(&url);
        self.apply_retry_flags(&mut cmd);

        self.enqueue(cmd, url, label);
    }

    /// Path to the music download directory, nested under `channels_root`
    /// alongside the platform folders.
    pub fn music_root(&self) -> PathBuf {
        self.channels_root.join("music")
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
            .arg("--xattrs");
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
        self.apply_retry_flags(&mut cmd);

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

    /// Enqueue a job that installs (or updates) the bgutil-pot binary
    /// and pip-installs the matching Python plugin into the bundled
    /// venv. Same UX shape as [`Self::start_ytdlp_update`] — progress
    /// streams into the Downloads modal.
    ///
    /// Before enqueueing we kill any already-running server child so
    /// the install can overwrite the binary in place without an "ETXTBSY"
    /// from the OS. The next download after install completes will
    /// re-spawn the server via [`Self::ensure_pot_server`].
    pub fn start_pot_provider_update(&mut self) {
        if let Some(mut child) = self.pot_server.take() {
            crate::pot_provider::kill_server(&mut child);
        }
        let cmd = crate::pot_provider::install_command();
        let url = "https://github.com/jim60105/bgutil-ytdlp-pot-provider-rs/releases/latest".to_string();
        let label = "install bgutil-pot + Python plugin".to_string();
        self.enqueue(cmd, url, label);
    }

    /// Spawn `cmd` on a background thread, streaming its output into a new [`Job`].
    fn spawn_job(&mut self, mut cmd: Command, url: String, label: String, retry_spec: Option<RetrySpec>, retry_count: u8, convert_on_finish: Option<ConvertSpec>) {
        // POT provider extractor-arg. yt-dlp lets us pass multiple
        // --extractor-args flags; this one points the bgutil plugin at
        // our local server. Only emitted when the user opted in *and*
        // the server child is actually running — yt-dlp would warn
        // about an unreachable base_url otherwise.
        if self.use_pot_provider && self.pot_server.is_some() {
            cmd.arg("--extractor-args")
                .arg(crate::pot_provider::extractor_args());
        }

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

        // Spawn the child *before* the reader thread so we can capture its
        // pid for the hang watchdog (the thread then owns the Child and
        // does the blocking wait()). On spawn failure, push a synthetic
        // failed job and bail.
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(err) => {
                self.push_synthetic_failure(
                    url, label, crate::error_class::ErrorClass::Other,
                    format!("could not launch yt-dlp: {err}"),
                );
                return;
            }
        };
        let child_pid = Some(child.id());

        thread::spawn(move || {
            let stderr_handle = child.stderr.take().map(|stderr| {
                let tx = tx.clone();
                let cookies_abs = cookies_abs.clone();
                thread::spawn(move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        let line = redact_sensitive(&line, &cookies_abs);
                        let _ = tx.send(Msg::Line(format!("[stderr] {line}")));
                    }
                })
            });

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
            // Wait for the stderr reader to flush all buffered lines before
            // announcing completion — otherwise Finished can race ahead of
            // trailing stderr output still in flight, and failure_class gets
            // computed from an incomplete log (see Job::drain).
            if let Some(h) = stderr_handle {
                let _ = h.join();
            }
            let _ = tx.send(Msg::Finished(ok));
        });

        self.jobs.push(Job {
            url,
            label,
            state: JobState::Running,
            progress: 0.0,
            log: VecDeque::new(),
            failure_class: None,
            retry_spec,
            retry_count,
            retry_handled: false,
            convert_on_finish,
            convert_handled: false,
            child_pid,
            last_activity: std::time::Instant::now(),
            watchdog_killed: false,
            cancelled: false,
            rx,
        });
    }

    /// Push a job that was never going to start — used for preflight
    /// failures (currently just disk-full). Constructs a closed channel
    /// so the immediate `Job::drain` sees no progress, and seeds the log
    /// with the human-readable reason so the UI's last_line + the error
    /// hint together explain what went wrong.
    fn push_synthetic_failure(
        &mut self,
        url: String,
        label: String,
        class: crate::error_class::ErrorClass,
        reason: String,
    ) {
        let (_tx, rx) = std::sync::mpsc::channel::<Msg>();
        // _tx is dropped here, so any subsequent drain hits the
        // "channel closed" branch and stays still. We start `state` as
        // Failed directly rather than running it through the Finished
        // transition.
        let mut log = VecDeque::new();
        log.push_back(format!("[preflight] {reason}"));
        self.jobs.push(Job {
            url,
            label,
            state: JobState::Failed,
            progress: 0.0,
            log,
            failure_class: Some(class),
            // Disk-full preflight isn't auto-retried — retrying without
            // freeing space would just fail again.
            retry_spec: None,
            retry_count: 0,
            retry_handled: true, // never retry a synthetic preflight failure
            convert_on_finish: None,
            convert_handled: true,
            child_pid: None,     // no process to watchdog
            last_activity: std::time::Instant::now(),
            watchdog_killed: false,
            cancelled: false,
            rx,
        });
    }

    /// Drain pending messages from all job threads and promote queued jobs.
    ///
    /// Call this regularly from the UI event loop to pick up progress updates.
    pub fn poll(&mut self) {
        for job in &mut self.jobs {
            job.drain();
        }
        self.check_watchdog();
        self.schedule_auto_retries();
        self.fire_due_retries();
        self.schedule_conversions();
        // Re-check after draining: finished jobs free slots for queued ones.
        self.promote_queued();
        // Once everything's idle, clear the adaptive backoff so the next
        // fresh batch starts at normal speed.
        if !self.any_running() && self.pending.is_empty() && self.retry_queue.is_empty() {
            self.rate_limited_backoff = false;
        }
    }

    /// Hang watchdog: SIGKILL any Running job that has produced no output
    /// or progress for [`HANG_TIMEOUT`]. Marks it `watchdog_killed` so the
    /// resulting failure is classified retryable (see [`Job::drain`]) and
    /// the auto-retry path gives it another go. The reader thread's
    /// `child.wait()` returns once the process dies, sending `Finished`
    /// normally — we don't reap here, just deliver the kill signal.
    fn check_watchdog(&mut self) {
        let now = std::time::Instant::now();
        for job in &mut self.jobs {
            if job.state != JobState::Running || job.watchdog_killed { continue; }
            let Some(pid) = job.child_pid else { continue };
            if now.duration_since(job.last_activity) < HANG_TIMEOUT { continue; }
            kill_pid(pid);
            job.watchdog_killed = true;
            job.last_activity = now; // don't re-kill before wait() returns
            job.log.push_back(format!(
                "⏱ watchdog: no output for {}s — killed (will auto-retry)",
                HANG_TIMEOUT.as_secs(),
            ));
        }
    }

    /// Scan freshly-Done download jobs that have a convert spec; for each
    /// `CONVERT_PATH:` line their yt-dlp run printed, enqueue an ffmpeg
    /// transcode job. Marks the job handled so it only fires once.
    fn schedule_conversions(&mut self) {
        let mut to_convert: Vec<(std::path::PathBuf, ConvertSpec)> = Vec::new();
        for job in &mut self.jobs {
            if job.convert_handled || job.state != JobState::Done { continue; }
            let Some(spec) = job.convert_on_finish.clone() else {
                job.convert_handled = true;
                continue;
            };
            job.convert_handled = true;
            for line in &job.log {
                // yt-dlp printed: "CONVERT_PATH:/abs/path/to/file.mkv"
                if let Some(p) = line.strip_prefix("CONVERT_PATH:") {
                    let path = std::path::PathBuf::from(p.trim());
                    if path.is_file() {
                        to_convert.push((path, spec.clone()));
                    }
                }
            }
        }
        for (path, spec) in to_convert {
            self.start_transcode(&path, &spec);
        }
    }

    /// Returns true if `class` warrants an automatic retry. Transient
    /// failures (rate-limit / captcha / network) are worth retrying; a
    /// removed video or missing codec is not — retrying changes nothing.
    fn is_retryable(class: crate::error_class::ErrorClass) -> bool {
        use crate::error_class::ErrorClass::*;
        matches!(class, RateLimited | NetworkError)
    }

    /// Scan freshly-failed jobs; schedule a delayed retry for retryable
    /// ones under the attempt cap, and engage the adaptive backoff on a
    /// rate-limit hit so the rest of the batch slows down.
    fn schedule_auto_retries(&mut self) {
        let now = std::time::Instant::now();
        let mut to_schedule: Vec<(RetrySpec, u8)> = Vec::new();
        for job in &mut self.jobs {
            if job.retry_handled || job.state != JobState::Failed { continue; }
            job.retry_handled = true;
            let Some(class) = job.failure_class else { continue };
            // A rate-limit hit slows the whole remaining batch.
            if class == crate::error_class::ErrorClass::RateLimited {
                self.rate_limited_backoff = true;
            }
            if !Self::is_retryable(class) { continue; }
            if job.retry_count >= MAX_AUTO_RETRIES { continue; }
            let Some(spec) = job.retry_spec.clone() else { continue };
            let next_attempt = job.retry_count + 1;
            to_schedule.push((spec, next_attempt));
            job.log.push_back(format!(
                "↻ auto-retry {next_attempt}/{MAX_AUTO_RETRIES} scheduled in {}s (transient {})",
                AUTO_RETRY_COOLDOWN.as_secs(),
                class.label(),
            ));
        }
        for (spec, attempt) in to_schedule {
            // Linear backoff: attempt 1 waits 1× cooldown, attempt 2 waits 2×.
            let delay = AUTO_RETRY_COOLDOWN * attempt as u32;
            self.retry_queue.push((now + delay, spec, attempt));
        }
    }

    /// Re-issue any retries whose cooldown has elapsed.
    fn fire_due_retries(&mut self) {
        let now = std::time::Instant::now();
        // Partition: due vs not-yet-due. Drain the due ones.
        let mut still_waiting = Vec::with_capacity(self.retry_queue.len());
        let due: Vec<(RetrySpec, u8)> = std::mem::take(&mut self.retry_queue)
            .into_iter()
            .filter_map(|(at, spec, attempt)| {
                if at <= now { Some((spec, attempt)) }
                else { still_waiting.push((at, spec, attempt)); None }
            })
            .collect();
        self.retry_queue = still_waiting;
        for (spec, attempt) in due {
            self.start_retry(spec, attempt);
        }
    }

    /// Rebuild + enqueue a retry of a previously-failed download. Re-runs
    /// `start()` to reconstruct the command identically; the attempt count
    /// rides through via `retry_attempt_override` so the new job knows it's
    /// a retry (and won't loop past the cap).
    fn start_retry(&mut self, spec: RetrySpec, attempt: u8) {
        self.retry_attempt_override = attempt;
        self.start(
            spec.url.clone(),
            &spec.info,
            spec.full_scan,
            spec.quality,
            spec.live,
            spec.opts.as_ref(),
        );
        self.retry_attempt_override = 0;
    }

    /// Build + enqueue an ffmpeg transcode of `src` per `spec`. The
    /// output lands next to the source; the source is renamed to
    /// `<stem>.original.<ext>` (kept) or deleted, depending on
    /// `spec.keep_original`. Runs as its own [`Job`] so the UI shows the
    /// transcode as a distinct phase.
    fn start_transcode(&mut self, src: &std::path::Path, spec: &ConvertSpec) {
        let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let parent = src.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
        let src_ext = src.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();

        // Target extension + ffmpeg codec args per mode.
        let (out_ext, codec_args): (&str, Vec<String>) = match spec.mode.as_str() {
            "remux-mp4" => ("mp4", vec![
                // Stream-copy: no re-encode, just re-container.
                "-c".into(), "copy".into(),
                // Some MKV audio (opus in mkv) isn't valid in mp4; let
                // ffmpeg fall back to aac only if copy fails would need a
                // second pass — keep it simple: copy video, aac audio.
                "-c:a".into(), "aac".into(),
            ]),
            "audio" => {
                let af = spec.audio_format.as_str();
                let mut a = vec!["-vn".into()]; // drop video
                // Pick a reasonable codec per container.
                match af {
                    "mp3"  => { a.extend(["-c:a".into(), "libmp3lame".into(), "-q:a".into(), "2".into()]); }
                    "m4a" | "aac" => { a.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "192k".into()]); }
                    "opus" => { a.extend(["-c:a".into(), "libopus".into(), "-b:a".into(), "160k".into()]); }
                    "flac" => { a.extend(["-c:a".into(), "flac".into()]); }
                    _      => { a.extend(["-c:a".into(), "libmp3lame".into(), "-q:a".into(), "2".into()]); }
                }
                (if af == "aac" { "m4a" } else { af }, a)
            }
            // Default "h264-mp4": re-encode to H.264 + AAC at the CRF.
            _ => ("mp4", vec![
                "-c:v".into(), "libx264".into(),
                "-crf".into(), spec.crf.to_string(),
                "-preset".into(), spec.preset.clone(),
                "-c:a".into(), "aac".into(),
                "-b:a".into(), "192k".into(),
                "-movflags".into(), "+faststart".into(),
            ]),
        };

        let out_path = parent.join(format!("{stem}.{out_ext}"));
        // If the source already has the target extension (e.g. remux-mp4
        // on an mp4), write to a temp name then swap, so we don't clobber
        // the input mid-encode.
        let same_ext = src_ext.eq_ignore_ascii_case(out_ext);
        let work_out = if same_ext {
            parent.join(format!("{stem}.converting.{out_ext}"))
        } else {
            out_path.clone()
        };

        // Build the ffmpeg command. -y overwrites a stale partial output.
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-hide_banner").arg("-loglevel").arg("error").arg("-stats")
            .arg("-y")
            .arg("-i").arg(src);
        for a in &codec_args { cmd.arg(a); }
        cmd.arg(&work_out);

        // Post-ffmpeg bookkeeping runs in the job thread via a wrapper
        // shell so the rename/delete happens only on ffmpeg success. We
        // encode it as a small bash script that runs ffmpeg then fixes up
        // the files.
        let label = format!("transcode → {out_ext}: {stem}");
        let url = src.display().to_string();
        self.enqueue_transcode(cmd, url, label, src.to_path_buf(), out_path, work_out, same_ext, spec.keep_original);
    }

    /// Enqueue a transcode job. We can't easily run post-ffmpeg file
    /// bookkeeping (rename original / swap temp) inside the generic
    /// spawn_job, so transcode jobs get their own lightweight runner
    /// thread that runs ffmpeg then does the file moves on success.
    fn enqueue_transcode(
        &mut self,
        mut cmd: Command,
        url: String,
        label: String,
        src: PathBuf,
        out_path: PathBuf,
        work_out: PathBuf,
        same_ext: bool,
        keep_original: bool,
    ) {
        let (tx, rx) = channel();
        // ffmpeg writes the encoded stream to `work_out` (a file) and its
        // diagnostics to stderr (via -loglevel error -stats), so stdout is
        // never used. Piping it without a reader would let a full 64KB pipe
        // buffer deadlock ffmpeg against our child.wait() below, so discard
        // it explicitly.
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
        // Spawn before the thread so the watchdog gets a pid (ffmpeg can
        // also hang on a bad input). On spawn failure, surface it.
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                self.push_synthetic_failure(
                    url, label, crate::error_class::ErrorClass::CodecMissing,
                    format!("could not launch ffmpeg: {e}"),
                );
                return;
            }
        };
        let child_pid = Some(child.id());
        thread::spawn(move || {
            let stderr_handle = child.stderr.take().map(|stderr| {
                let tx = tx.clone();
                thread::spawn(move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        let _ = tx.send(Msg::Line(format!("[ffmpeg] {line}")));
                    }
                })
            });
            let ok = matches!(child.wait(), Ok(s) if s.success());
            // See the analogous join in spawn_job: wait for stderr to finish
            // flushing before any Finished-triggered classification runs.
            if let Some(h) = stderr_handle {
                let _ = h.join();
            }
            if ok {
                // File bookkeeping on success.
                if same_ext {
                    // src and out share an extension. Keep or drop the
                    // original, then move the temp output into place.
                    if keep_original {
                        let orig = src.with_extension(format!("original.{}",
                            src.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()));
                        let _ = std::fs::rename(&src, &orig);
                    } else {
                        let _ = std::fs::remove_file(&src);
                    }
                    let _ = std::fs::rename(&work_out, &out_path);
                } else {
                    // Different extensions: output already at out_path.
                    if keep_original {
                        let orig = src.with_extension(format!("original.{}",
                            src.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()));
                        let _ = std::fs::rename(&src, &orig);
                    } else {
                        let _ = std::fs::remove_file(&src);
                    }
                }
                let _ = tx.send(Msg::Line(format!("✓ wrote {}", out_path.display())));
            } else {
                // Clean up a partial temp output on failure.
                if same_ext { let _ = std::fs::remove_file(&work_out); }
            }
            let _ = tx.send(Msg::Finished(ok));
        });
        self.jobs.push(Job {
            url,
            label,
            state: JobState::Running,
            progress: 0.0,
            log: VecDeque::new(),
            failure_class: None,
            retry_spec: None,        // don't auto-retry transcodes
            retry_count: 0,
            retry_handled: true,
            convert_on_finish: None, // a transcode doesn't itself convert
            convert_handled: true,
            child_pid,
            last_activity: std::time::Instant::now(),
            watchdog_killed: false,
            cancelled: false,
            rx,
        });
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

    /// Cancel a running job by index: SIGKILL its process and mark it
    /// `cancelled` so the auto-retry path skips it and the UI shows a
    /// distinct "cancelled" state rather than a misleading error class.
    /// The reader thread's `wait()` returns once the process dies and
    /// delivers `Finished(false)` normally, transitioning it to Failed.
    /// No-op for non-running jobs. Returns whether a job was cancelled.
    pub fn cancel_job(&mut self, idx: usize) -> bool {
        let Some(job) = self.jobs.get_mut(idx) else { return false };
        if job.state != JobState::Running { return false; }
        if let Some(pid) = job.child_pid {
            kill_pid(pid);
        }
        job.cancelled = true;
        job.retry_handled = true; // a user cancel must not auto-retry
        job.last_activity = std::time::Instant::now();
        job.log.push_back("⛔ cancelled by user".to_string());
        true
    }

    /// Cancel a still-queued (not-yet-started) job by index into the pending
    /// queue. Returns whether an entry was removed.
    pub fn cancel_queued(&mut self, idx: usize) -> bool {
        if idx < self.pending.len() {
            self.pending.remove(idx);
            true
        } else {
            false
        }
    }

    /// Manually re-issue a finished (failed/cancelled/done) job by index,
    /// reusing the inputs captured at start time. Resets the attempt counter
    /// so the user-triggered retry gets a fresh auto-retry budget. The old
    /// job row is removed. No-op (returns false) for a running job or one
    /// with no captured `retry_spec` (e.g. live recordings, self-update).
    pub fn retry_job(&mut self, idx: usize) -> bool {
        let Some(job) = self.jobs.get(idx) else { return false };
        if job.state == JobState::Running { return false; }
        let Some(spec) = job.retry_spec.clone() else { return false };
        self.jobs.remove(idx);
        self.start_retry(spec, 0);
        true
    }

    /// Full log buffer for a job by index (for the UI's per-job log view).
    pub fn job_log(&self, idx: usize) -> Option<Vec<String>> {
        self.jobs.get(idx).map(|j| j.log.iter().cloned().collect())
    }
}

impl Drop for Downloader {
    /// Tear down the bgutil-pot child if we spawned one. Without this
    /// the server keeps running after catacomb exits — orphaned and
    /// still bound to port 4416, blocking the next launch from
    /// re-spawning.
    fn drop(&mut self) {
        if let Some(mut child) = self.pot_server.take() {
            crate::pot_provider::kill_server(&mut child);
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

/// SIGKILL a child process by pid (used by the hang watchdog). The reader
/// thread's `child.wait()` reaps it; we only deliver the signal here.
#[cfg(unix)]
fn kill_pid(pid: u32) {
    // SAFETY: libc::kill with a valid pid + signal is sound; an invalid
    // pid just returns ESRCH which we ignore.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
}

/// Non-Unix fallback: no portable pid-kill, so the watchdog can flag the
/// stall but can't force-terminate. (Windows isn't a first-class target.)
#[cfg(not(unix))]
fn kill_pid(_pid: u32) {}

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
        let abs = "/home/luna/.config/catacomb/cookies.txt";
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

    // ── Auto-retry classification ────────────────────────────────────────
    use crate::error_class::ErrorClass;
    #[test]
    fn retryable_only_for_transient_classes() {
        assert!(Downloader::is_retryable(ErrorClass::RateLimited));
        assert!(Downloader::is_retryable(ErrorClass::NetworkError));
        // Permanent / user-action-required classes must NOT auto-retry.
        assert!(!Downloader::is_retryable(ErrorClass::NotFound));
        assert!(!Downloader::is_retryable(ErrorClass::MembersOnly));
        assert!(!Downloader::is_retryable(ErrorClass::Geoblocked));
        assert!(!Downloader::is_retryable(ErrorClass::CodecMissing));
        assert!(!Downloader::is_retryable(ErrorClass::DiskFull));
        assert!(!Downloader::is_retryable(ErrorClass::BadCookies));
        assert!(!Downloader::is_retryable(ErrorClass::Other));
    }

    #[test]
    fn adaptive_backoff_triples_sleeps() {
        let mut d = Downloader::new(
            std::path::PathBuf::from("/tmp"), "none".into(), 1, false, false);
        // Normal: 1/2/6.
        let mut cmd = Command::new("yt-dlp");
        d.apply_retry_flags(&mut cmd);
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.windows(2).any(|w| w == ["--sleep-requests", "1"]));
        assert!(args.windows(2).any(|w| w == ["--max-sleep-interval", "6"]));
        // After a rate-limit hit: backoff engaged → 3/8/20.
        d.rate_limited_backoff = true;
        let mut cmd2 = Command::new("yt-dlp");
        d.apply_retry_flags(&mut cmd2);
        let args2: Vec<String> = cmd2.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args2.windows(2).any(|w| w == ["--sleep-requests", "3"]));
        assert!(args2.windows(2).any(|w| w == ["--max-sleep-interval", "20"]));
    }

    // ── Convert resolver ─────────────────────────────────────────────────
    use crate::config::ConvertSection;
    use crate::download_options::DownloadOptions as DO;

    #[test]
    fn convert_off_when_global_empty_and_no_override() {
        let g = ConvertSection::default(); // mode = ""
        assert!(ConvertSpec::resolve(&g, None).is_none());
    }

    #[test]
    fn convert_global_h264_defaults_filled() {
        let g = ConvertSection { mode: "h264-mp4".into(), crf: 0, preset: String::new(),
            audio_format: String::new(), keep_original: false };
        let s = ConvertSpec::resolve(&g, None).unwrap();
        assert_eq!(s.mode, "h264-mp4");
        assert_eq!(s.crf, 23);           // 0 → default 23
        assert_eq!(s.preset, "medium");  // empty → default
    }

    #[test]
    fn convert_per_channel_off_overrides_global_on() {
        let g = ConvertSection { mode: "h264-mp4".into(), ..Default::default() };
        let opts = DO { convert_mode: Some("off".into()), ..Default::default() };
        assert!(ConvertSpec::resolve(&g, Some(&opts)).is_none());
    }

    #[test]
    fn convert_per_channel_mode_wins() {
        let g = ConvertSection { mode: "h264-mp4".into(), ..Default::default() };
        let opts = DO { convert_mode: Some("audio".into()), ..Default::default() };
        let s = ConvertSpec::resolve(&g, Some(&opts)).unwrap();
        assert_eq!(s.mode, "audio");
        assert_eq!(s.audio_format, "mp3"); // empty global → default mp3
    }

    // ── Hang watchdog ────────────────────────────────────────────────────
    #[test]
    fn watchdog_killed_job_fails_retryable() {
        // A job the watchdog killed leaves no error line; drain() must
        // force a retryable class so auto-retry re-queues it (a hang is
        // transient), rather than classify() returning Other (terminal).
        let (tx, rx) = channel();
        let mut job = Job {
            url: "u".into(), label: "l".into(), state: JobState::Running,
            progress: 0.0, log: VecDeque::new(), failure_class: None,
            retry_spec: None, retry_count: 0, retry_handled: false,
            convert_on_finish: None, convert_handled: false,
            child_pid: Some(999_999),
            last_activity: std::time::Instant::now(),
            watchdog_killed: true,
            cancelled: false,
            rx,
        };
        tx.send(Msg::Finished(false)).unwrap();
        job.drain();
        assert_eq!(job.state, JobState::Failed);
        assert_eq!(job.failure_class, Some(crate::error_class::ErrorClass::NetworkError));
        assert!(Downloader::is_retryable(job.failure_class.unwrap()));
    }

    #[test]
    fn normal_failure_still_classified_from_log() {
        // Without watchdog_killed, drain() classifies from the log as before.
        let (tx, rx) = channel();
        let mut job = Job {
            url: "u".into(), label: "l".into(), state: JobState::Running,
            progress: 0.0, log: VecDeque::new(), failure_class: None,
            retry_spec: None, retry_count: 0, retry_handled: false,
            convert_on_finish: None, convert_handled: false,
            child_pid: Some(1), last_activity: std::time::Instant::now(),
            watchdog_killed: false, cancelled: false, rx,
        };
        tx.send(Msg::Line("ERROR: Video unavailable. This video has been removed".into())).unwrap();
        tx.send(Msg::Finished(false)).unwrap();
        job.drain();
        assert_eq!(job.failure_class, Some(crate::error_class::ErrorClass::NotFound));
    }

    // ── Subtitle resolver ────────────────────────────────────────────────
    fn dl_with_subs(subs: crate::config::SubtitlesSection) -> Downloader {
        let mut d = Downloader::new(
            std::path::PathBuf::from("/tmp"), "none".into(), 1, false, false);
        d.subtitle_defaults = subs;
        d
    }
    fn args_of(d: &Downloader, opts: Option<&DownloadOptions>) -> Vec<String> {
        let mut cmd = Command::new("yt-dlp");
        d.apply_subtitle_flags(&mut cmd, opts);
        cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn subs_global_defaults_emit_write_and_auto() {
        let d = dl_with_subs(crate::config::SubtitlesSection::default());
        let args = args_of(&d, None);
        assert!(args.contains(&"--write-subs".to_string()));
        assert!(args.contains(&"--write-auto-subs".to_string()));
        // No langs/format/embed by default.
        assert!(!args.contains(&"--embed-subs".to_string()));
        assert!(!args.iter().any(|a| a == "--convert-subs"));
    }

    #[test]
    fn subs_disabled_emits_nothing() {
        let mut g = crate::config::SubtitlesSection::default();
        g.enabled = false;
        let d = dl_with_subs(g);
        assert!(args_of(&d, None).is_empty());
    }

    #[test]
    fn subs_global_format_embed_langs() {
        let g = crate::config::SubtitlesSection {
            enabled: true, auto_generated: false, embed: true,
            format: "srt".into(), langs: "en,ja".into(),
        };
        let d = dl_with_subs(g);
        let args = args_of(&d, None);
        assert!(args.contains(&"--write-subs".to_string()));
        assert!(!args.contains(&"--write-auto-subs".to_string())); // auto off
        assert!(args.windows(2).any(|w| w == ["--sub-langs", "en,ja"]));
        assert!(args.windows(2).any(|w| w == ["--convert-subs", "srt"]));
        assert!(args.contains(&"--embed-subs".to_string()));
    }

    #[test]
    fn subs_per_channel_overrides_global() {
        // Global: enabled+auto. Channel: force OFF.
        let d = dl_with_subs(crate::config::SubtitlesSection::default());
        let opts = DownloadOptions { subtitles_enabled: Some(false), ..Default::default() };
        assert!(args_of(&d, Some(&opts)).is_empty());

        // Global: auto on. Channel: auto off + embed on + own langs.
        let opts2 = DownloadOptions {
            subtitles_auto: Some(false),
            subtitles_embed: Some(true),
            subtitle_langs: vec!["en".into()],
            subtitle_format: Some("vtt".into()),
            ..Default::default()
        };
        let args = args_of(&d, Some(&opts2));
        assert!(!args.contains(&"--write-auto-subs".to_string()));
        assert!(args.contains(&"--embed-subs".to_string()));
        assert!(args.windows(2).any(|w| w == ["--sub-langs", "en"]));
        assert!(args.windows(2).any(|w| w == ["--convert-subs", "vtt"]));
    }
    // URL classification tests live in `platform` now — see its tests module.
}
