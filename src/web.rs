//! HTTP server providing a browser-based library UI and download API.
//!
//! # Architecture
//!
//! All mutable server state lives in [`WebState`], wrapped in `Arc<WebState>`
//! and shared across axum handlers.  Mutable fields use `Mutex` or `AtomicBool`
//! so they can be updated by concurrent requests without blocking the async
//! runtime.
//!
//! The server can be started in two ways:
//!
//! * **Standalone mode** — `serve(config)` blocks until the process exits.
//! * **GUI-embedded mode** — `run_with_shutdown(config)` spawns a background
//!   Tokio runtime and returns a `Sender<()>` the GUI can use to stop the server.
//!
//! # AGPL §13 compliance
//!
//! This software is licensed under the GNU Affero General Public License v3.
//! Any deployment that serves the web UI to network users must make the
//! Corresponding Source available.  Set `web.source_url` in `config.toml` to
//! a URL where the source can be obtained; the URL is displayed in the UI footer
//! and returned by `GET /api/settings`.

use std::collections::{HashMap, HashSet};
use std::path::{Path as StdPath, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::{
    body::{Body, Bytes},
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tower_http::services::ServeDir;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use argon2::password_hash::SaltString;

use crate::config::Config;
use crate::database::Database;
use crate::downloader::{detect_url_kind, Downloader, JobState};
use crate::library;
use crate::maintenance;

// ── Shared state ──────────────────────────────────────────────────────────────

/// Serialisable snapshot of a single download job, sent to the browser.
#[derive(Clone, Serialize)]
pub struct JobSnapshot {
    pub label: String,
    pub url: String,
    /// One of `"running"`, `"done"`, or `"failed"`.
    pub state: &'static str,
    pub progress: f32,
    pub last_line: String,
}

/// All mutable state shared across axum handlers via `Arc<WebState>`.
pub struct WebState {
    /// Scanned channel/playlist/video tree, refreshed after each completed download.
    pub library: Mutex<Vec<library::Channel>>,
    /// Active and recently finished yt-dlp jobs.
    pub downloader: Mutex<Downloader>,
    /// Set of video IDs the user has marked as watched (persisted in SQLite).
    pub watched: Mutex<HashSet<String>>,
    /// Last known playback position per video ID in seconds (persisted in SQLite).
    pub positions: Mutex<HashMap<String, f64>>,
    /// Shared SQLite connection, opened once at startup instead of per request.
    pub db: Mutex<Database>,
    pub channels_root: PathBuf,
    pub config_path: PathBuf,
    pub config: Mutex<Config>,
    /// Whether to transcode MKV→mp4 on the fly for playback (requires ffmpeg).
    pub transcode: AtomicBool,
    /// Active session tokens. Non-empty only when a password is set; a valid
    /// `session` cookie matching one of these grants access to the gated UI/API.
    pub sessions: Mutex<HashSet<String>>,
}

impl WebState {
    fn job_snapshots(dl: &Downloader) -> Vec<JobSnapshot> {
        dl.jobs
            .iter()
            .map(|j| JobSnapshot {
                label: j.label.clone(),
                url: j.url.clone(),
                state: match j.state {
                    JobState::Running => "running",
                    JobState::Done => "done",
                    JobState::Failed => "failed",
                },
                progress: j.progress,
                last_line: j.log.last().cloned().unwrap_or_default(),
            })
            .collect()
    }
}

// ── API types ─────────────────────────────────────────────────────────────────
// These types are serialised to JSON and consumed by the browser UI.

/// Response body for `GET /api/library`.
#[derive(Serialize)]
struct LibraryResponse {
    channels: Vec<ChannelInfo>,
}

/// JSON representation of a single channel sent to the browser.
#[derive(Serialize)]
struct ChannelInfo {
    name: String,
    total_videos: usize,
    size_bytes: u64,
    subscriber_count: Option<u64>,
    uploader: Option<String>,
    channel_url: Option<String>,
    playlists: Vec<PlaylistInfo>,
    videos: Vec<VideoInfo>,
}

/// JSON representation of a playlist within a channel.
#[derive(Serialize)]
struct PlaylistInfo {
    name: String,
    videos: Vec<VideoInfo>,
}

/// JSON representation of a single video sent to the browser.
///
/// `resume_pos` is only set when the user has played more than 3 seconds
/// so that the "continue watching" list stays meaningful.
#[derive(Serialize)]
struct VideoInfo {
    id: String,
    title: String,
    duration_secs: Option<f64>,
    file_size: Option<u64>,
    has_video: bool,
    has_live_chat: bool,
    watched: bool,
    video_url: Option<String>,
    thumb_url: Option<String>,
    subtitles: Vec<SubtitleInfo>,
    has_chapters: bool,
    resume_pos: Option<f64>,
}

/// A single subtitle track URL for a video.
#[derive(Serialize)]
struct SubtitleInfo {
    lang: String,
    /// Human-readable label (e.g. "English"), shown in the track selector.
    label: String,
    url: String,
}

/// Request body for `POST /api/download`.
#[derive(Deserialize)]
struct StartDownloadRequest {
    url: String,
    /// When true, omits `--break-on-existing` so every video is checked
    /// individually — slower but fills gaps in partially-archived channels.
    #[serde(default)]
    full_scan: bool,
}

/// Response body for `GET /api/progress`.
#[derive(Serialize)]
struct ProgressResponse {
    jobs: Vec<JobSnapshot>,
}

#[derive(Serialize, Deserialize)]
struct SettingsPayload {
    transcode: bool,
    /// URL of the source repository, injected by the server for AGPL §13 compliance.
    /// Clients MUST NOT send this field; the server ignores it on POST.
    #[serde(skip_deserializing, default)]
    source_url: Option<String>,
    /// Current binding address and port, sent by server only.
    #[serde(skip_deserializing, default)]
    current_bind: Option<String>,
    /// List of available bind options, sent by server only.
    #[serde(skip_deserializing, default)]
    available_binds: Option<Vec<BindOption>>,
    /// Selected bind mode (localhost, tailscale, lan, all). Clients can send this on POST to change.
    #[serde(skip_deserializing, default)]
    bind_mode: Option<String>,
    /// Whether a password is required for downloads (sent by server only).
    #[serde(skip_deserializing, default)]
    download_password_required: bool,
    /// New plaintext password to set for downloads. Clients send this on POST; server does not return it.
    #[serde(skip_serializing, default)]
    new_download_password: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct BindOption {
    pub id: String,
    pub label: String,
    pub address: String,
}

// Build a `/files/<rel>` URL from an absolute path, percent-encoding each segment.
fn file_url(channels_root: &StdPath, full: &StdPath) -> Option<String> {
    let rel = full.strip_prefix(channels_root).ok()?;
    let mut parts: Vec<String> = Vec::new();
    for c in rel.components() {
        if let std::path::Component::Normal(s) = c {
            parts.push(percent_encode_segment(s.to_str()?));
        }
    }
    if parts.is_empty() { return None; }
    Some(format!("/files/{}", parts.join("/")))
}

fn percent_encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn lang_label(code: &str) -> String {
    let base = code.split('-').next().unwrap_or(code);
    let name = match base {
        "en" => "English",
        "es" => "Spanish",
        "fr" => "French",
        "de" => "German",
        "ja" => "Japanese",
        "ko" => "Korean",
        "zh" => "Chinese",
        "pt" => "Portuguese",
        "ru" => "Russian",
        "it" => "Italian",
        "ar" => "Arabic",
        "hi" => "Hindi",
        "nl" => "Dutch",
        "pl" => "Polish",
        "tr" => "Turkish",
        "sv" => "Swedish",
        "id" => "Indonesian",
        "vi" => "Vietnamese",
        "th" => "Thai",
        _ => return code.to_string(),
    };
    if code.ends_with("-orig") || code.contains("auto") {
        format!("{name} (auto)")
    } else {
        name.to_string()
    }
}

fn find_video_info_path(library: &[library::Channel], id: &str) -> Option<PathBuf> {
    for ch in library {
        for v in ch.videos.iter().chain(ch.playlists.iter().flat_map(|p| p.videos.iter())) {
            if v.id == id {
                return v.info_path.clone();
            }
        }
    }
    None
}

fn find_video_path(library: &[library::Channel], id: &str) -> Option<PathBuf> {
    for ch in library {
        for v in ch.videos.iter().chain(ch.playlists.iter().flat_map(|p| p.videos.iter())) {
            if v.id == id {
                return v.video_path.clone();
            }
        }
    }
    None
}

fn detect_tailscale_ip() -> Option<String> {
    if std::path::Path::new("/proc/net/if_inet6").exists() || std::path::Path::new("/etc/tailscale").exists() {
        if let Ok(output) = std::process::Command::new("hostname")
            .arg("-I")
            .output() {
            let ip_str = String::from_utf8_lossy(&output.stdout);
            ip_str
                .split_whitespace()
                .find(|ip| ip.starts_with("100."))
                .map(|s| s.to_string())
        } else {
            None
        }
    } else {
        None
    }
}

fn detect_lan_ip() -> Option<String> {
    if let Ok(output) = std::process::Command::new("hostname")
        .arg("-I")
        .output() {
        let ip_str = String::from_utf8_lossy(&output.stdout);
        ip_str
            .split_whitespace()
            .find(|ip| !ip.starts_with("127.") && !ip.starts_with("100."))
            .map(|s| s.to_string())
    } else {
        None
    }
}

pub fn get_available_binds(port: u16) -> Vec<BindOption> {
    let mut opts = vec![
        BindOption {
            id: "localhost".to_string(),
            label: "Localhost only".to_string(),
            address: format!("127.0.0.1:{port}"),
        },
    ];

    if let Some(ts_ip) = detect_tailscale_ip() {
        opts.push(BindOption {
            id: "tailscale".to_string(),
            label: format!("Tailscale ({})", ts_ip),
            address: format!("{ts_ip}:{port}"),
        });
    }

    if let Some(lan_ip) = detect_lan_ip() {
        if lan_ip != "127.0.0.1" {
            opts.push(BindOption {
                id: "lan".to_string(),
                label: format!("LAN ({})", lan_ip),
                address: format!("{lan_ip}:{port}"),
            });
        }
    }

    opts.push(BindOption {
        id: "all".to_string(),
        label: "All interfaces (0.0.0.0)".to_string(),
        address: format!("0.0.0.0:{port}"),
    });

    opts
}

pub fn resolve_bind_mode(mode: &str) -> String {
    match mode {
        "tailscale" => detect_tailscale_ip().unwrap_or_else(|| "127.0.0.1".to_string()),
        "lan" => detect_lan_ip().unwrap_or_else(|| "127.0.0.1".to_string()),
        "all" => "0.0.0.0".to_string(),
        _ => "127.0.0.1".to_string(),
    }
}

/// Infer the bind-mode id (`localhost`/`tailscale`/`lan`/`all`) from a stored bind address.
pub fn bind_mode_of(addr: &str) -> &'static str {
    match addr {
        "127.0.0.1" | "localhost" => "localhost",
        "0.0.0.0" => "all",
        a if a.starts_with("100.") => "tailscale",
        _ => "lan",
    }
}

// ── Cookies ─────────────────────────────────────────────────────────────────────

/// Convert SubRip (SRT) subtitle text to WebVTT.
///
/// The only structural differences are the `WEBVTT` header and the timestamp
/// decimal separator (SRT uses `,`, VTT uses `.`).
fn srt_to_vtt(srt: &str) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for line in srt.lines() {
        if line.contains("-->") {
            out.push_str(&line.replace(',', "."));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// Path to the `cookies.txt` yt-dlp reads, resolved against the process working
/// directory (the same place `config.toml` lives and where the downloader's
/// relative `--cookies cookies.txt` resolves).
pub fn cookies_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("cookies.txt")
}

/// Count cookie entries (Netscape lines with 7 tab-separated fields).
fn count_cookies(text: &str) -> usize {
    text.lines().filter(|l| l.split('\t').count() >= 7).count()
}

/// Whether a cookies file exists and how many cookie entries it holds.
pub fn cookies_status() -> (bool, usize) {
    match std::fs::read_to_string(cookies_path()) {
        Ok(s) => (true, count_cookies(&s)),
        Err(_) => (false, 0),
    }
}

/// Validate that `text` looks like a Netscape cookie jar and write it to
/// [`cookies_path`]. Returns the number of cookie entries written, or an error
/// message if the content doesn't look like a cookies.txt.
pub fn write_cookies(text: &str) -> Result<usize, String> {
    if text.trim().is_empty() {
        return Err("no cookies provided".to_string());
    }
    let has_cookie_line = text.lines().any(|l| l.split('\t').count() >= 7);
    let has_header = text.trim_start().starts_with("# Netscape")
        || text.trim_start().starts_with("# HTTP Cookie File");
    if !has_cookie_line && !has_header {
        return Err(
            "doesn't look like a Netscape cookies.txt (expected tab-separated fields)".to_string(),
        );
    }
    let mut content = text.to_string();
    if !content.ends_with('\n') {
        content.push('\n');
    }
    std::fs::write(cookies_path(), &content).map_err(|e| e.to_string())?;
    Ok(count_cookies(&content))
}

pub fn hash_password(password: &str) -> Option<String> {
    use rand::thread_rng;
    let salt = SaltString::generate(thread_rng());
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .ok()
        .map(|hash| hash.to_string())
}

fn verify_password(password: &str, hash: &str) -> bool {
    if let Ok(parsed_hash) = PasswordHash::new(hash) {
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok()
    } else {
        false
    }
}

// ── Session auth ────────────────────────────────────────────────────────────────

/// Generate a 256-bit random session token, hex-encoded.
fn generate_session_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Extract the `session` cookie value from request headers, if present.
fn session_token_from_headers(headers: &HeaderMap) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie
        .split(';')
        .filter_map(|p| p.trim().strip_prefix("session="))
        .next()
        .map(|s| s.to_string())
}

/// True if the request carries a valid session cookie.
fn is_authed(state: &WebState, headers: &HeaderMap) -> bool {
    match session_token_from_headers(headers) {
        Some(token) => state.sessions.lock().unwrap().contains(&token),
        None => false,
    }
}

/// Whether a download/access password is configured.
fn password_required(state: &WebState) -> bool {
    state.db.lock().unwrap().get_setting("password_hash").ok().flatten().is_some()
}

#[derive(Deserialize)]
struct LoginRequest {
    password: String,
}

/// `POST /api/login` — verify the password and issue a session cookie.
async fn post_login(
    State(state): State<Arc<WebState>>,
    Json(body): Json<LoginRequest>,
) -> Response {
    let hash = state.db.lock().unwrap().get_setting("password_hash").ok().flatten();
    let Some(hash) = hash else {
        // No password configured; nothing to authenticate against.
        return (StatusCode::OK, "no password set").into_response();
    };
    if !verify_password(&body.password, &hash) {
        return (StatusCode::UNAUTHORIZED, "invalid password").into_response();
    }
    let token = generate_session_token();
    state.sessions.lock().unwrap().insert(token.clone());
    let cookie = format!("session={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age=2592000");
    ([(header::SET_COOKIE, cookie)], StatusCode::OK).into_response()
}

/// `POST /api/logout` — invalidate the current session and clear the cookie.
async fn post_logout(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
) -> Response {
    if let Some(token) = session_token_from_headers(&headers) {
        state.sessions.lock().unwrap().remove(&token);
    }
    let cookie = "session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0";
    ([(header::SET_COOKIE, cookie)], StatusCode::OK).into_response()
}

/// Middleware gating every route behind a session cookie when a password is set.
/// With no password configured, all requests pass through unchanged (preserves
/// the localhost-only default). `/api/login` is always reachable so users can
/// authenticate; unauthenticated `GET /` is served a login page instead of the app.
async fn auth_middleware(
    State(state): State<Arc<WebState>>,
    req: Request,
    next: Next,
) -> Response {
    if !password_required(&state) {
        return next.run(req).await;
    }
    let path = req.uri().path();
    if path == "/api/login" {
        return next.run(req).await;
    }
    if is_authed(&state, req.headers()) {
        return next.run(req).await;
    }
    if path == "/" {
        return (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            LOGIN_HTML,
        )
            .into_response();
    }
    (StatusCode::UNAUTHORIZED, "authentication required").into_response()
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn get_index() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        HTML_UI,
    )
}

async fn get_library(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let lib = state.library.lock().unwrap();
    let watched = state.watched.lock().unwrap();
    let positions = state.positions.lock().unwrap();
    let root = state.channels_root.as_path();
    let transcode = state.transcode.load(Ordering::Relaxed);

    let to_info = |v: &library::Video, watched: &HashSet<String>| {
        let video_url = v.video_path.as_deref().and_then(|p| {
            if transcode {
                Some(format!("/api/transcode/{}", percent_encode_segment(&v.id)))
            } else {
                file_url(root, p)
            }
        });
        let thumb_url = v.thumb_path.as_deref().and_then(|p| file_url(root, p));
        let subtitles: Vec<SubtitleInfo> = v.subtitles.iter()
            .filter_map(|s| {
                let is_srt = s.path.extension().and_then(|e| e.to_str()) == Some("srt");
                let url = if is_srt {
                    // Route SRT through the on-the-fly conversion endpoint.
                    let rel = s.path.strip_prefix(root).ok()?;
                    Some(format!("/api/sub-vtt/{}", rel.display()))
                } else {
                    file_url(root, &s.path)
                }?;
                Some(SubtitleInfo {
                    lang: s.lang.clone(),
                    label: lang_label(&s.lang),
                    url,
                })
            })
            .collect();
        let resume_pos = positions.get(&v.id).copied().filter(|&p| p > 3.0);
        VideoInfo {
            id: v.id.clone(),
            title: v.title.clone(),
            duration_secs: v.duration_secs,
            file_size: v.file_size,
            has_video: v.video_path.is_some(),
            has_live_chat: v.has_live_chat,
            watched: watched.contains(&v.id),
            video_url,
            thumb_url,
            subtitles,
            has_chapters: v.has_chapters,
            resume_pos,
        }
    };

    let channels = lib.iter().map(|ch| {
        ChannelInfo {
            name: ch.name.clone(),
            total_videos: ch.total_videos(),
            size_bytes: ch.total_size_cached,
            subscriber_count: ch.meta.as_ref().and_then(|m| m.subscriber_count),
            uploader: ch.meta.as_ref().and_then(|m| m.uploader.clone()),
            channel_url: ch.meta.as_ref().and_then(|m| m.channel_url.clone()),
            playlists: ch.playlists.iter().map(|p| PlaylistInfo {
                name: p.name.clone(),
                videos: p.videos.iter().map(|v| to_info(v, &watched)).collect(),
            }).collect(),
            videos: ch.videos.iter().map(|v| to_info(v, &watched)).collect(),
        }
    }).collect();

    Json(LibraryResponse { channels })
}

async fn get_progress(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let mut dl = state.downloader.lock().unwrap();
    dl.poll();
    Json(ProgressResponse { jobs: WebState::job_snapshots(&dl) })
}

async fn post_download(
    State(state): State<Arc<WebState>>,
    Json(body): Json<StartDownloadRequest>,
) -> impl IntoResponse {
    // Access control is handled centrally by auth_middleware; reaching here
    // means the request is authenticated (or no password is configured).
    let url = body.url.trim().to_string();
    if url.is_empty() {
        return (StatusCode::BAD_REQUEST, "empty URL").into_response();
    }
    let kind = detect_url_kind(&url);
    state.downloader.lock().unwrap().start(url, &kind, body.full_scan);
    (StatusCode::ACCEPTED, "ok").into_response()
}

async fn post_watched(
    State(state): State<Arc<WebState>>,
    Path(video_id): Path<String>,
) -> impl IntoResponse {
    let db = state.db.lock().unwrap();
    let mut watched = state.watched.lock().unwrap();
    let now_watched = !watched.contains(&video_id);
    if let Err(e) = db.set_watched(&video_id, now_watched) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    if now_watched { watched.insert(video_id); } else { watched.remove(&video_id); }
    (StatusCode::OK, if now_watched { "watched" } else { "unwatched" }).into_response()
}

async fn get_settings(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let cfg = state.config.lock().unwrap();
    let source_url = cfg.web.source_url.clone();
    let port = cfg.web.port;
    let current_bind = cfg.web.bind.clone();
    let available_binds = get_available_binds(port);
    drop(cfg);
    let download_password_required =
        state.db.lock().unwrap().get_setting("password_hash").ok().flatten().is_some();
    Json(SettingsPayload {
        transcode: state.transcode.load(Ordering::Relaxed),
        source_url,
        current_bind: Some(format!("{}:{}", current_bind, port)),
        available_binds: Some(available_binds),
        bind_mode: None,
        download_password_required,
        new_download_password: None,
    })
}

async fn post_settings(
    State(state): State<Arc<WebState>>,
    Json(body): Json<SettingsPayload>,
) -> impl IntoResponse {
    state.transcode.store(body.transcode, Ordering::Relaxed);
    let mut cfg = state.config.lock().unwrap();
    cfg.web.transcode = body.transcode;

    if let Some(new_mode) = &body.bind_mode {
        let new_addr = resolve_bind_mode(new_mode);
        cfg.web.bind = new_addr;
    }

    if let Err(e) = cfg.save(&state.config_path) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("save failed: {e}")).into_response();
    }
    let source_url = cfg.web.source_url.clone();
    let current_bind = cfg.web.bind.clone();
    let port = cfg.web.port;
    let available_binds = get_available_binds(port);
    drop(cfg);

    if let Some(new_pwd) = &body.new_download_password {
        let db = state.db.lock().unwrap();
        let result = if new_pwd.is_empty() {
            db.set_setting("password_hash", None)
        } else if let Some(hashed) = hash_password(new_pwd) {
            db.set_setting("password_hash", Some(&hashed))
        } else {
            return (StatusCode::INTERNAL_SERVER_ERROR, "failed to hash password").into_response();
        };
        if let Err(e) = result {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")).into_response();
        }
        // Password changed: drop all existing sessions so they must re-authenticate.
        state.sessions.lock().unwrap().clear();
    }

    let download_password_required =
        state.db.lock().unwrap().get_setting("password_hash").ok().flatten().is_some();
    Json(SettingsPayload {
        transcode: body.transcode,
        source_url,
        current_bind: Some(format!("{}:{}", current_bind, port)),
        available_binds: Some(available_binds),
        bind_mode: None,
        download_password_required,
        new_download_password: None,
    }).into_response()
}

/// `GET /api/sub-vtt/*path` — serve an SRT subtitle file as WebVTT.
///
/// The path is relative to the channels root.  The file must be within the
/// channels root (path traversal is rejected with 403).
async fn get_sub_vtt(
    State(state): State<Arc<WebState>>,
    Path(rel): Path<String>,
) -> Response {
    let path = state.channels_root.join(&rel);
    // Reject path traversal outside the library.
    let ok = match (state.channels_root.canonicalize(), path.canonicalize()) {
        (Ok(root), Ok(p)) => p.starts_with(root),
        _ => false,
    };
    if !ok {
        return StatusCode::FORBIDDEN.into_response();
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let vtt = srt_to_vtt(&content);
    ([(header::CONTENT_TYPE, "text/vtt; charset=utf-8")], vtt).into_response()
}

async fn get_transcode(
    State(state): State<Arc<WebState>>,
    Path(id): Path<String>,
) -> Response {
    let path = {
        let lib = state.library.lock().unwrap();
        find_video_path(&lib, &id)
    };
    let Some(path) = path else {
        return (StatusCode::NOT_FOUND, "no video").into_response();
    };

    let mut cmd = tokio::process::Command::new("ffmpeg");
    cmd.arg("-hide_banner")
        .arg("-loglevel").arg("error")
        .arg("-i").arg(&path)
        .arg("-c:v").arg("libx264")
        .arg("-preset").arg("veryfast")
        .arg("-crf").arg("23")
        .arg("-c:a").arg("aac")
        .arg("-b:a").arg("128k")
        .arg("-movflags").arg("frag_keyframe+empty_moov+default_base_moof")
        .arg("-f").arg("mp4")
        .arg("-")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("ffmpeg spawn failed: {e}")).into_response(),
    };
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => return (StatusCode::INTERNAL_SERVER_ERROR, "no stdout from ffmpeg").into_response(),
    };

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(8);
    tokio::spawn(async move {
        let _child_guard = child; // dropped at task end → kills ffmpeg
        use tokio::io::AsyncReadExt;
        let mut stdout = stdout;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match stdout.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(Ok(Bytes::copy_from_slice(&buf[..n]))).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    break;
                }
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "video/mp4")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(stream))
        .unwrap()
}

#[derive(serde::Deserialize)]
struct ResumeBody { position: f64 }

async fn post_resume(
    State(state): State<Arc<WebState>>,
    Path(video_id): Path<String>,
    Json(body): Json<ResumeBody>,
) -> impl IntoResponse {
    let db = state.db.lock().unwrap();
    if body.position > 3.0 {
        let _ = db.set_position(&video_id, body.position);
        state.positions.lock().unwrap().insert(video_id, body.position);
    } else {
        let _ = db.clear_position(&video_id);
        state.positions.lock().unwrap().remove(&video_id);
    }
    (StatusCode::OK, "ok")
}

#[derive(serde::Deserialize)]
struct PreviewQuery { url: String }

async fn get_preview(
    State(_state): State<Arc<WebState>>,
    Query(q): Query<PreviewQuery>,
) -> impl IntoResponse {
    let url = q.url.trim().to_string();
    if url.is_empty() {
        return (StatusCode::BAD_REQUEST, "no url").into_response();
    }
    let output = tokio::process::Command::new("yt-dlp")
        .arg("--dump-single-json")
        .arg("--flat-playlist")
        .arg("--no-warnings")
        .arg("--cookies").arg("cookies.txt")
        .arg("--impersonate").arg("Chrome-146:Macos-26")
        .arg(&url)
        .stdin(Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await;
    let output = match output {
        Ok(o) if !o.stdout.is_empty() => o,
        Ok(_) => return (StatusCode::BAD_REQUEST, "yt-dlp returned no data").into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("yt-dlp failed: {e}")).into_response(),
    };
    let val: serde_json::Value = match serde_json::from_slice(&output.stdout) {
        Ok(v) => v,
        Err(_) => return (StatusCode::BAD_REQUEST, "could not parse yt-dlp output").into_response(),
    };
    let is_playlist = val.get("_type").and_then(|t| t.as_str()).map_or(false, |t| t == "playlist");
    let entry_count = val.get("entries").and_then(|e| e.as_array()).map(|a| a.len()).unwrap_or(0);
    let preview = serde_json::json!({
        "type": if is_playlist { "channel/playlist" } else { "video" },
        "title": val.get("title").and_then(|t| t.as_str()),
        "channel": val.get("channel").or_else(|| val.get("uploader")).and_then(|t| t.as_str()),
        "thumbnail": val.get("thumbnail").and_then(|t| t.as_str()),
        "duration": val.get("duration").and_then(|d| d.as_f64()),
        "view_count": val.get("view_count").and_then(|v| v.as_u64()),
        "entry_count": entry_count,
    });
    Json(preview).into_response()
}

async fn post_clear_jobs(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    state.downloader.lock().unwrap().clear_finished();
    (StatusCode::OK, "cleared")
}

async fn post_remove_job(
    State(state): State<Arc<WebState>>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    state.downloader.lock().unwrap().remove_job(idx);
    (StatusCode::OK, "removed")
}

async fn get_chapters(
    State(state): State<Arc<WebState>>,
    Path(video_id): Path<String>,
) -> impl IntoResponse {
    let info_path = {
        let lib = state.library.lock().unwrap();
        find_video_info_path(&lib, &video_id)
    };
    let Some(info_path) = info_path else {
        return (StatusCode::NOT_FOUND, "no info.json").into_response();
    };
    let Ok(text) = std::fs::read_to_string(&info_path) else {
        return (StatusCode::OK, Json(serde_json::json!({"chapters": []}))).into_response();
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) else {
        return (StatusCode::OK, Json(serde_json::json!({"chapters": []}))).into_response();
    };
    let chapters = val.get("chapters")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter().filter_map(|ch| {
                let title = ch.get("title").and_then(|t| t.as_str())?.to_string();
                let start = ch.get("start_time").and_then(|t| t.as_f64())?;
                let end = ch.get("end_time").and_then(|t| t.as_f64());
                Some(serde_json::json!({"title": title, "start": start, "end": end}))
            }).collect::<Vec<_>>()
        })
        .unwrap_or_default();
    (StatusCode::OK, Json(serde_json::json!({"chapters": chapters}))).into_response()
}

async fn get_metadata(
    State(state): State<Arc<WebState>>,
    Path(video_id): Path<String>,
) -> impl IntoResponse {
    let info_path = {
        let lib = state.library.lock().unwrap();
        find_video_info_path(&lib, &video_id)
    };
    let Some(info_path) = info_path else {
        return (StatusCode::NOT_FOUND, "no info.json").into_response();
    };
    match std::fs::read_to_string(&info_path) {
        Ok(text) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            text,
        ).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_description(
    State(state): State<Arc<WebState>>,
    Path(video_id): Path<String>,
) -> impl IntoResponse {
    let lib = state.library.lock().unwrap();
    for ch in lib.iter() {
        for v in ch.videos.iter().chain(ch.playlists.iter().flat_map(|p| p.videos.iter())) {
            if v.id == video_id {
                let text = v.description_path.as_ref()
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .unwrap_or_default();
                return (StatusCode::OK, [(header::CONTENT_TYPE, "text/plain; charset=utf-8")], text).into_response();
            }
        }
    }
    (StatusCode::NOT_FOUND, "not found").into_response()
}

async fn post_rescan(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let new_lib = library::scan_channels(&state.channels_root);
    // Refresh watched from DB after rescan
    if let Ok(w) = state.db.lock().unwrap().get_watched() {
        *state.watched.lock().unwrap() = w;
    }
    *state.library.lock().unwrap() = new_lib;
    (StatusCode::OK, "rescanned")
}

/// `GET /api/maintenance/scan` — report duplicate videos and missing assets.
async fn get_maintenance_scan(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let lib = state.library.lock().unwrap();
    let report = maintenance::scan(&state.channels_root, &lib);
    Json(report)
}

#[derive(Deserialize)]
struct RemoveRequest {
    paths: Vec<PathBuf>,
}

/// `POST /api/maintenance/remove` — delete the listed duplicate files.
/// Paths outside the library root are refused.
async fn post_maintenance_remove(
    State(state): State<Arc<WebState>>,
    Json(body): Json<RemoveRequest>,
) -> impl IntoResponse {
    let (removed, errors) = maintenance::remove_files(&state.channels_root, &body.paths);
    // Refresh the library so the removed copies disappear from the UI.
    let new_lib = library::scan_channels(&state.channels_root);
    *state.library.lock().unwrap() = new_lib;
    Json(serde_json::json!({ "removed": removed, "errors": errors }))
}

/// `POST /api/maintenance/repair/:id` — re-fetch missing sidecars for one video.
async fn post_maintenance_repair(
    State(state): State<Arc<WebState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let target = {
        let lib = state.library.lock().unwrap();
        maintenance::locate(&lib, &id)
    };
    let Some((dir, stem)) = target else {
        return (StatusCode::NOT_FOUND, "video not found in library").into_response();
    };
    state.downloader.lock().unwrap().repair(&id, &dir, &stem);
    (StatusCode::ACCEPTED, "repair queued").into_response()
}

/// Delete cookies.txt, removing all stored session cookies.
pub fn clear_cookies() -> Result<(), String> {
    let p = cookies_path();
    if p.exists() {
        std::fs::remove_file(&p).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `GET /api/cookies` — report whether a cookies file exists and its entry count.
async fn get_cookies() -> impl IntoResponse {
    let (exists, count) = cookies_status();
    Json(serde_json::json!({ "exists": exists, "cookies": count }))
}

#[derive(Deserialize)]
struct CookiesBody {
    cookies: String,
}

/// `POST /api/cookies` — replace cookies.txt with pasted Netscape-format content.
async fn post_cookies(Json(body): Json<CookiesBody>) -> impl IntoResponse {
    match write_cookies(&body.cookies) {
        Ok(count) => Json(serde_json::json!({ "ok": true, "cookies": count })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// `DELETE /api/cookies` — remove cookies.txt entirely.
async fn delete_cookies() -> impl IntoResponse {
    match clear_cookies() {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn run(config: Config) -> ! {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = tx; // Keep tx alive to prevent rx from becoming permanently closed
    rt.block_on(serve(config, rx));
    unreachable!()
}

pub fn run_with_shutdown(config: Config) -> std::sync::mpsc::Sender<()> {
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(serve(config, shutdown_rx));
    });
    shutdown_tx
}

async fn serve(config: Config, shutdown_rx: std::sync::mpsc::Receiver<()>) {
    let channels_root = config.backup.directory.clone();
    let _ = std::fs::create_dir_all(&channels_root);
    let db_path = channels_root.join("yt-offline.db");
    let config_path = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("config.toml");

    let library = library::scan_channels(&channels_root);
    let db = Database::open(&db_path)
        .unwrap_or_else(|_| Database::open_in_memory().expect("in-memory db failed"));
    let watched = db.get_watched().unwrap_or_default();
    let positions = db.get_positions().unwrap_or_default();

    let downloader = Downloader::new(channels_root.clone(), config.player.browser.clone());
    let transcode = AtomicBool::new(config.web.transcode);
    let port = config.web.port;
    let bind_addr = config.web.bind.clone();

    let state = Arc::new(WebState {
        library: Mutex::new(library),
        downloader: Mutex::new(downloader),
        watched: Mutex::new(watched),
        positions: Mutex::new(positions),
        db: Mutex::new(db),
        channels_root: channels_root.clone(),
        config_path,
        config: Mutex::new(config),
        transcode,
        sessions: Mutex::new(HashSet::new()),
    });

    let app = Router::new()
        .route("/", get(get_index))
        .route("/api/library", get(get_library))
        .route("/api/progress", get(get_progress))
        .route("/api/download", post(post_download))
        .route("/api/watched/:id", post(post_watched))
        .route("/api/resume/:id", post(post_resume))
        .route("/api/preview", get(get_preview))
        .route("/api/rescan", post(post_rescan))
        .route("/api/jobs/clear", post(post_clear_jobs))
        .route("/api/jobs/:idx", axum::routing::delete(post_remove_job))
        .route("/api/description/:id", get(get_description))
        .route("/api/chapters/:id", get(get_chapters))
        .route("/api/metadata/:id", get(get_metadata))
        .route("/api/settings", get(get_settings).post(post_settings))
        .route("/api/transcode/:id", get(get_transcode))
        .route("/api/sub-vtt/*path", get(get_sub_vtt))
        .route("/api/maintenance/scan", get(get_maintenance_scan))
        .route("/api/maintenance/remove", post(post_maintenance_remove))
        .route("/api/maintenance/repair/:id", post(post_maintenance_repair))
        .route("/api/cookies", get(get_cookies).post(post_cookies).delete(delete_cookies))
        .route("/api/login", post(post_login))
        .route("/api/logout", post(post_logout))
        .nest_service("/files", ServeDir::new(&channels_root))
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware))
        .with_state(state);

    let listener = match tokio::net::TcpListener::bind(format!("{bind_addr}:{port}")).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Cannot bind to {bind_addr}:{port}: {e}");
            return;
        }
    };
    println!("yt-offline web UI: http://localhost:{port}");

    let server = axum::serve(listener, app);
    tokio::select! {
        _ = server => {},
        _ = tokio::task::spawn_blocking(move || {
            let _ = shutdown_rx.recv();
        }) => {
            println!("Web server shutting down");
        }
    }
}

// ── Embedded UI ───────────────────────────────────────────────────────────────

/// Standalone login page served at `GET /` when a password is set and the
/// request is unauthenticated. Posts to `/api/login` and reloads on success.
const LOGIN_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>yt-offline — Sign in</title>
<style>
  *{box-sizing:border-box;margin:0;padding:0}
  body{background:#1a1a2e;color:#eee;font:14px/1.5 system-ui,sans-serif;display:flex;align-items:center;justify-content:center;height:100vh}
  .box{background:#16213e;border:1px solid #334;border-radius:8px;padding:28px;width:300px;display:flex;flex-direction:column;gap:12px}
  h1{font-size:1.1em;text-align:center}
  input{background:#0f3460;border:1px solid #334;color:#eee;padding:9px 11px;border-radius:4px;font-size:14px}
  button{background:#e94560;border:none;color:#fff;padding:9px;border-radius:4px;cursor:pointer;font-size:14px;font-weight:600}
  .err{color:#f87171;font-size:12px;text-align:center;min-height:16px}
</style>
</head>
<body>
<div class="box">
  <h1>yt-offline</h1>
  <input type="password" id="pwd" placeholder="Password" autofocus onkeydown="if(event.key==='Enter')login()">
  <button onclick="login()">Sign in</button>
  <div class="err" id="err"></div>
</div>
<script>
'use strict';
async function login(){
  const pwd=document.getElementById('pwd').value;
  const err=document.getElementById('err');
  err.textContent='';
  try{
    const r=await fetch('/api/login',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({password:pwd})});
    if(r.ok){location.reload()}else{err.textContent='Invalid password'}
  }catch{err.textContent='Connection error'}
}
</script>
</body>
</html>"#;

const HTML_UI: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>yt-offline</title>
<style>
  :root{--bg:#1a1a2e;--panel:#16213e;--card:#0f3460;--accent:#e94560;--text:#eee;--muted:#999;--border:#334}
  .theme-light{--bg:#f5f5f5;--panel:#fff;--card:#e8e8e8;--accent:#e94560;--text:#111;--muted:#666;--border:#ccc}
  .theme-solarized{--bg:#002b36;--panel:#073642;--card:#004052;--accent:#268bd2;--text:#839496;--muted:#586e75;--border:#144}
  .theme-nord{--bg:#2e3440;--panel:#3b4252;--card:#434c5e;--accent:#88c0d0;--text:#eceff4;--muted:#9aa;--border:#4c566a}
  .theme-amoled{--bg:#000;--panel:#0a0a0a;--card:#111;--accent:#e94560;--text:#eee;--muted:#666;--border:#222}
  .theme-dracula{--bg:#282a36;--panel:#282a36;--card:#343746;--accent:#bd93f9;--text:#f8f8f2;--muted:#6272a4;--border:#44475a}
  .theme-trans{--bg:#e8f7fd;--panel:#fef0f4;--card:#fce8f2;--accent:#55cdfc;--text:#cc0066;--muted:#888;--border:#f7a8b8}
  .theme-emo-nocturnal{--bg:#0a0a0a;--panel:#0d0d0d;--card:#1a1a1a;--accent:#ff0090;--text:#e8e8e8;--muted:#888;--border:#2a2a2a}
  .theme-emo-coffin{--bg:#0d0009;--panel:#110010;--card:#1a0018;--accent:#cc2222;--text:#c0c0c0;--muted:#666;--border:#3a0030}
  .theme-emo-scene-queen{--bg:#080818;--panel:#0a0a1e;--card:#111128;--accent:#39ff14;--text:#c8c8ff;--muted:#666;--border:#222244}
  *{box-sizing:border-box;margin:0;padding:0}
  body{background:var(--bg);color:var(--text);font:14px/1.5 system-ui,sans-serif;display:flex;flex-direction:column;height:100vh;overflow:hidden}
  header{background:var(--panel);padding:8px 12px;display:flex;gap:8px;align-items:center;border-bottom:1px solid var(--border);flex-shrink:0;flex-wrap:wrap}
  header h1{font-size:1em;font-weight:700;white-space:nowrap}
  #search{flex:1;min-width:100px;background:var(--bg);border:1px solid var(--border);color:var(--text);padding:5px 9px;border-radius:4px;font-size:13px}
  #sort{background:var(--card);color:var(--text);border:1px solid var(--border);padding:4px 6px;border-radius:4px;font-size:12px}
  #hdr-stats{font-size:11px;color:var(--muted);white-space:nowrap}
  #status{font-size:11px;color:var(--muted);white-space:nowrap}
  button{background:var(--card);color:var(--text);border:1px solid var(--border);padding:5px 10px;border-radius:4px;cursor:pointer;font-size:12px;touch-action:manipulation;white-space:nowrap}
  button:hover,button:active{background:var(--accent);border-color:var(--accent);color:#fff}
  button.primary{background:var(--accent);border-color:var(--accent);color:#fff}
  #menu-btn{display:none;font-size:18px;padding:3px 9px;line-height:1}
  main{display:flex;flex:1;overflow:hidden;position:relative}
  aside{width:210px;background:var(--panel);border-right:1px solid var(--border);overflow-y:auto;flex-shrink:0;padding:6px 0;transition:transform .2s}
  #sidebar-overlay{display:none;position:fixed;inset:0;background:rgba(0,0,0,.5);z-index:49}
  .sidebar-label{padding:2px 10px;font-size:10px;text-transform:uppercase;color:var(--muted);letter-spacing:.07em;margin-top:6px}
  .ch-item{padding:6px 10px;cursor:pointer;font-size:13px;border-left:3px solid transparent;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  .ch-item:hover{background:var(--card)}
  .ch-item.active{border-left-color:var(--accent);background:rgba(128,128,128,.1)}
  .ch-sub{padding:4px 10px 4px 22px;font-size:12px;color:var(--muted);cursor:pointer;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  .ch-sub:hover{color:var(--text);background:rgba(128,128,128,.08)}
  .ch-sub.active{color:var(--text)}
  section#content{flex:1;overflow-y:auto;padding:10px;min-width:0}
  .toolbar{display:flex;align-items:center;gap:8px;margin-bottom:8px;flex-wrap:wrap}
  .grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(180px,1fr));gap:10px}
  .card{background:var(--card);border-radius:6px;overflow:hidden;border:2px solid transparent;transition:border-color .15s;display:flex;flex-direction:column;cursor:pointer}
  .card:hover{border-color:rgba(233,69,96,.5)}
  .card.selected{border-color:var(--accent)}
  .card.watched{opacity:.55}
  .card-thumb{position:relative;width:100%;aspect-ratio:16/9;background:#000;overflow:hidden}
  .card-thumb img{width:100%;height:100%;object-fit:cover;display:block}
  .card-thumb .nothumb{display:flex;align-items:center;justify-content:center;width:100%;height:100%;color:var(--muted);font-size:11px}
  .card-thumb .dur{position:absolute;bottom:4px;right:4px;background:rgba(0,0,0,.75);color:#fff;font-size:11px;padding:1px 5px;border-radius:3px;font-weight:600}
  .card-thumb .resume-bar{position:absolute;bottom:0;left:0;height:3px;background:var(--accent)}
  .card-title{font-size:13px;font-weight:600;padding:7px 8px 2px;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;min-height:34px}
  .card-meta{font-size:11px;color:var(--muted);padding:0 8px 4px;display:flex;gap:5px;flex-wrap:wrap}
  .card-foot{padding:4px 8px 8px;display:flex;gap:6px}
  .card-foot button{font-size:11px;padding:4px 8px}
  .card-foot .play{background:var(--accent);border-color:var(--accent);color:#fff;flex:1}
  .modal-bg{position:fixed;inset:0;background:rgba(0,0,0,.85);display:flex;align-items:center;justify-content:center;z-index:100;padding:10px}
  .modal{background:var(--panel);border:1px solid var(--border);border-radius:8px;padding:14px;max-width:95vw;max-height:95vh;display:flex;flex-direction:column;gap:10px;width:100%;overflow:hidden}
  .modal-hdr{display:flex;align-items:center;gap:8px;flex-shrink:0}
  .modal-hdr h2{flex:1;font-size:14px;font-weight:600;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .modal-body{display:flex;gap:12px;overflow:hidden;flex:1;min-height:0}
  .modal video{flex:1;min-width:0;max-height:75vh;background:#000}
  .chapters-pane{width:200px;background:var(--bg);border:1px solid var(--border);border-radius:4px;overflow-y:auto;flex-shrink:0}
  .chapters-pane h4{font-size:11px;padding:7px 10px;color:var(--muted);text-transform:uppercase;letter-spacing:.05em;border-bottom:1px solid var(--border);position:sticky;top:0;background:var(--bg)}
  .chapter{padding:6px 10px;font-size:12px;cursor:pointer;border-bottom:1px solid var(--border)}
  .chapter:hover{background:var(--card)}
  .chapter.active{background:var(--card);border-left:3px solid var(--accent)}
  .chapter .ch-time{color:var(--muted);font-size:11px;font-family:monospace}
  .chapter .ch-title{font-weight:500;margin-top:1px}
  .settings-row{display:flex;align-items:center;gap:8px;padding:6px 0}
  .settings-row label{flex:1;font-size:13px}
  .settings-hint{font-size:11px;color:var(--muted)}
  #details{background:var(--panel);border-top:2px solid var(--accent);padding:10px 14px;flex-shrink:0;max-height:200px;overflow-y:auto;display:none}
  .det-head{display:flex;align-items:flex-start;gap:8px;margin-bottom:4px}
  .det-head h3{flex:1;font-size:13px;font-weight:600;margin:0}
  .det-actions{display:flex;gap:5px;flex-shrink:0;flex-wrap:wrap}
  .det-meta{font-size:12px;color:var(--muted);margin-bottom:6px;display:flex;gap:8px;flex-wrap:wrap}
  .det-desc{font-size:12px;color:var(--muted);white-space:pre-wrap;overflow-y:auto;max-height:90px}
  .meta-grid{display:grid;grid-template-columns:max-content 1fr;gap:5px 12px;font-size:12px;padding:4px}
  .meta-grid dt{color:var(--muted);font-weight:600;white-space:nowrap}
  .meta-grid dd{color:var(--text);word-break:break-word;overflow-wrap:anywhere}
  .meta-grid dd a{color:var(--accent);text-decoration:none}
  .meta-grid dd a:hover{text-decoration:underline}
  .meta-tabs{display:flex;gap:4px;border-bottom:1px solid var(--border);margin-bottom:6px}
  .meta-tab{padding:4px 10px;cursor:pointer;font-size:12px;border:1px solid transparent;border-bottom:none;border-radius:4px 4px 0 0}
  .meta-tab.active{background:var(--card);border-color:var(--border)}
  .meta-raw{font-family:monospace;font-size:11px;background:var(--bg);padding:10px;border-radius:4px;white-space:pre;overflow:auto;max-height:60vh}
  #jobs{background:var(--panel);border-top:1px solid var(--border);flex-shrink:0;max-height:40vh;overflow-y:auto}
  .job{display:flex;align-items:center;gap:8px;padding:5px 14px;font-size:12px;border-bottom:1px solid var(--border);flex-wrap:wrap;min-width:0}
  .badge{font-weight:700;min-width:48px;flex-shrink:0}
  .badge.running{color:#facc15}.badge.done{color:#4ade80}.badge.failed{color:#f87171}
  progress{flex:1;height:5px;accent-color:var(--accent);min-width:40px}
  footer{background:var(--panel);border-top:1px solid var(--border);padding:8px 12px;display:flex;gap:8px;align-items:center;flex-shrink:0}
  footer input{flex:1;min-width:0;background:var(--bg);border:1px solid var(--border);color:var(--text);padding:5px 9px;border-radius:4px;font-size:13px}
  .preview-thumb{width:100%;max-width:280px;aspect-ratio:16/9;object-fit:cover;border-radius:4px;background:#000;flex-shrink:0}
  .empty{text-align:center;color:var(--muted);padding:40px;font-size:13px}
  @media(max-width:640px){
    #menu-btn{display:block}
    aside{position:fixed;top:0;left:0;height:100%;z-index:50;transform:translateX(-100%);width:240px}
    aside.open{transform:translateX(0)}
    #sidebar-overlay.open{display:block}
    #hdr-stats,#sort{display:none}
    .grid{grid-template-columns:repeat(auto-fill,minmax(150px,1fr));gap:8px}
    #details{max-height:160px}
    .chapters-pane{display:none}
    .modal video{max-height:55vh}
    section#content{padding:8px}
    footer{padding:6px 10px}
  }
</style>
</head>
<body>
<header>
  <button id="menu-btn" onclick="toggleSidebar()">☰</button>
  <h1>yt-offline</h1>
  <input type="search" id="search" placeholder="Filter…" oninput="renderGrid()">
  <select id="sort" onchange="renderGrid()">
    <option value="title">Title</option>
    <option value="dur-asc">Shortest</option>
    <option value="dur-desc">Longest</option>
    <option value="size-asc">Smallest</option>
    <option value="size-desc">Largest</option>
  </select>
  <span id="hdr-stats"></span>
  <button onclick="rescan()" title="Rescan library">⟳</button>
  <button onclick="openMaintenance()" title="Library health">🩺</button>
  <button onclick="openSettings()">⚙</button>
  <span id="status"></span>
</header>
<div id="sidebar-overlay" onclick="closeSidebar()"></div>
<main>
  <aside id="sidebar"></aside>
  <section id="content">
    <div class="toolbar">
      <label style="font-size:12px;color:var(--muted)"><input type="checkbox" id="bulk" onchange="toggleBulk()"> Select</label>
      <span id="bulk-actions" style="display:none;gap:6px">
        <button onclick="bulkWatched(true)">✓ Mark watched</button>
        <button onclick="bulkWatched(false)">○ Unwatch</button>
        <span id="sel-count" style="font-size:12px;color:var(--muted)"></span>
      </span>
    </div>
    <div class="grid" id="grid"></div>
  </section>
</main>
<div id="details"></div>
<div id="jobs"></div>
<footer>
  <input type="url" id="dl-url" placeholder="YouTube URL…" onkeydown="if(event.key==='Enter')previewDownload()">
  <button class="primary" onclick="previewDownload()">⬇ Download</button>
  <label style="display:flex;align-items:center;gap:4px;font-size:12px;white-space:nowrap;cursor:pointer" title="Check every video individually instead of stopping at the first already-archived one. Slower but fills gaps."><input type="checkbox" id="dl-full-scan"> Full scan</label>
  <span id="agpl-notice" style="font-size:10px;color:var(--muted);margin-left:auto;white-space:nowrap;overflow:hidden;text-overflow:ellipsis"></span>
</footer>

<script>
'use strict';
let library=[], channelUrls=[], activeChannelIdx=null, activePlaylistIdx=null, showContinue=false;
let bulkMode=false, selected=new Set(), selectedId=null;
let currentPlayingId=null, saveTimer=null;

/* ── Sidebar (mobile) ──────────────────────────────────────────── */
function toggleSidebar(){document.getElementById('sidebar').classList.toggle('open');document.getElementById('sidebar-overlay').classList.toggle('open')}
function closeSidebar(){document.getElementById('sidebar').classList.remove('open');document.getElementById('sidebar-overlay').classList.remove('open')}

/* ── API ────────────────────────────────────────────────────────── */
async function api(path,opts){const r=await fetch(path,opts);if(!r.ok)throw new Error(await r.text());return r}

/* ── Library ────────────────────────────────────────────────────── */
async function loadLibrary(){
  try{
    const data=(await(await api('/api/library')).json());
    library=data.channels;
    channelUrls=library.map(ch=>ch.channel_url||null);
    const total=library.reduce((s,c)=>s+c.size_bytes,0);
    document.getElementById('hdr-stats').textContent=total>0?fmtSize(total)+' total':'';
    renderSidebar();renderGrid();
    if(selectedId)renderDetails();
  }catch(e){setStatus('Error: '+e.message)}
}
function setStatus(s){document.getElementById('status').textContent=s}

/* ── Sidebar ────────────────────────────────────────────────────── */
function renderSidebar(){
  const el=document.getElementById('sidebar');
  const allVids=library.flatMap(ch=>[...ch.videos,...ch.playlists.flatMap(p=>p.videos)]);
  const contVids=allVids.filter(v=>v.resume_pos&&v.resume_pos>5&&!v.watched);
  const total=library.reduce((s,c)=>s+c.total_videos,0);
  let h=`<div class="sidebar-label">Library</div>`;
  if(contVids.length)h+=`<div class="ch-item${showContinue?' active':''}" onclick="setContinue()">▶ Continue (${contVids.length})</div>`;
  h+=`<div class="ch-item${!showContinue&&activeChannelIdx===null?' active':''}" onclick="setView(null,null)">⊞ All (${total})</div>`;
  h+=`<div class="sidebar-label" style="margin-top:8px">Channels</div>`;
  for(let i=0;i<library.length;i++){
    const ch=library[i];
    const active=activeChannelIdx===i&&activePlaylistIdx===null&&!showContinue;
    const size=ch.size_bytes>0?' · '+fmtSize(ch.size_bytes):'';
    h+=`<div class="ch-item${active?' active':''}" onclick="setView(${i},null)">${esc(ch.name)} (${ch.total_videos}${size})</div>`;
    if(activeChannelIdx===i&&!showContinue){
      for(let pi=0;pi<ch.playlists.length;pi++){
        const pl=ch.playlists[pi];
        h+=`<div class="ch-sub${activePlaylistIdx===pi?' active':''}" onclick="setView(${i},${pi})">└ ${esc(pl.name)} (${pl.videos.length})</div>`;
      }
      h+=`<div class="ch-sub" style="color:var(--accent)" onclick="downloadChannelByIdx(${i})">⬇ Check for new videos</div>`;
    }
  }
  el.innerHTML=h;
}
function setContinue(){showContinue=true;activeChannelIdx=null;activePlaylistIdx=null;selected.clear();closeSidebar();renderSidebar();renderGrid()}
function setView(ci,pi){showContinue=false;activeChannelIdx=ci;activePlaylistIdx=pi;selected.clear();closeSidebar();renderSidebar();renderGrid()}

/* ── Grid ───────────────────────────────────────────────────────── */
function currentVideos(){
  const q=document.getElementById('search').value.toLowerCase();
  const sort=document.getElementById('sort').value;
  let vids=[];
  if(showContinue){
    for(const ch of library)
      for(const v of[...ch.videos,...ch.playlists.flatMap(p=>p.videos)])
        if(v.resume_pos&&v.resume_pos>5&&!v.watched&&(!q||v.title.toLowerCase().includes(q)||v.id.includes(q)))
          vids.push({...v,channel:ch.name});
    vids.sort((a,b)=>(b.resume_pos||0)-(a.resume_pos||0));
    return vids;
  }
  for(let i=0;i<library.length;i++){
    const ch=library[i];
    if(activeChannelIdx!==null&&i!==activeChannelIdx)continue;
    const pool=activePlaylistIdx!==null?(ch.playlists[activePlaylistIdx]?.videos||[]):[...ch.videos,...ch.playlists.flatMap(p=>p.videos)];
    for(const v of pool)
      if(!q||v.title.toLowerCase().includes(q)||v.id.includes(q))vids.push({...v,channel:ch.name});
  }
  if(sort==='dur-asc')vids.sort((a,b)=>(a.duration_secs??0)-(b.duration_secs??0));
  if(sort==='dur-desc')vids.sort((a,b)=>(b.duration_secs??0)-(a.duration_secs??0));
  if(sort==='size-asc')vids.sort((a,b)=>(a.file_size??0)-(b.file_size??0));
  if(sort==='size-desc')vids.sort((a,b)=>(b.file_size??0)-(a.file_size??0));
  if(sort==='title')vids.sort((a,b)=>a.title.localeCompare(b.title));
  return vids;
}

function renderGrid(){
  const vids=currentVideos();
  setStatus(vids.length+' video'+(vids.length!==1?'s':''));
  const grid=document.getElementById('grid');
  if(!vids.length){grid.innerHTML='<div class="empty">Nothing here.</div>';return}
  const showChCol=activeChannelIdx===null&&!showContinue;
  grid.innerHTML=vids.map(v=>{
    const chk=bulkMode?`<input type="checkbox" ${selected.has(v.id)?'checked':''} onchange="toggleSel('${v.id}',this.checked)">`:'';
    const meta=[
      showChCol?esc(v.channel):null,
      v.duration_secs!=null?fmtDur(v.duration_secs):null,
      v.file_size!=null?fmtSize(v.file_size):null,
      v.has_live_chat?'💬':null,
      !v.has_video?'<span style="color:#f87171">no file</span>':null,
    ].filter(Boolean).join(' · ');
    const thumb=v.thumb_url?`<img src="${v.thumb_url}" loading="lazy" alt="">`:'<div class="nothumb">no thumbnail</div>';
    const dur=v.duration_secs!=null?`<span class="dur">${fmtDur(v.duration_secs)}</span>`:'';
    const resumeBar=v.resume_pos&&v.duration_secs?`<div class="resume-bar" style="width:${Math.min(100,v.resume_pos/v.duration_secs*100).toFixed(1)}%"></div>`:'';
    const playBtn=v.has_video&&v.video_url?`<button class="play" onclick="playVideo('${v.id}')">▶ Play</button>`:'';
    return `<div class="card${v.watched?' watched':''}${selectedId===v.id?' selected':''}" onclick="selectVideo('${v.id}')">
      <div class="card-thumb">${thumb}${dur}${resumeBar}</div>
      <div class="card-title">${chk} ${esc(v.title)}</div>
      <div class="card-meta">${meta||'&nbsp;'}</div>
      <div class="card-foot" onclick="event.stopPropagation()">
        ${playBtn}
        <button onclick="toggleWatched('${v.id}')">${v.watched?'✓':'○'}</button>
      </div>
    </div>`;
  }).join('');
}

/* ── Watched / bulk ─────────────────────────────────────────────── */
async function toggleWatched(id){try{await api('/api/watched/'+id,{method:'POST'});await loadLibrary()}catch(e){setStatus('Error: '+e.message)}}
function toggleBulk(){bulkMode=document.getElementById('bulk').checked;selected.clear();document.getElementById('bulk-actions').style.display=bulkMode?'flex':'none';renderGrid()}
function toggleSel(id,on){if(on)selected.add(id);else selected.delete(id);document.getElementById('sel-count').textContent=selected.size+' selected'}
async function bulkWatched(on){await Promise.all([...selected].map(id=>api('/api/watched/'+id,{method:'POST'})));selected.clear();await loadLibrary()}

/* ── Download preview ───────────────────────────────────────────── */
async function previewDownload(){
  const url=document.getElementById('dl-url').value.trim();
  if(!url)return;
  setStatus('Fetching preview…');
  const bg=document.createElement('div');bg.className='modal-bg';
  bg.innerHTML=`<div class="modal" style="max-width:420px">
    <div class="modal-hdr"><h2>Fetching info…</h2></div>
    <div style="color:var(--muted);font-size:13px">Contacting YouTube, please wait…</div>
    <button onclick="this.closest('.modal-bg').remove();setStatus('')">Cancel</button>
  </div>`;
  document.body.appendChild(bg);
  try{
    const r=await fetch('/api/preview?url='+encodeURIComponent(url));
    if(!r.ok)throw new Error(await r.text());
    const d=await r.json();
    const info=[d.type,d.entry_count?d.entry_count+' videos':null,d.duration?fmtDur(d.duration):null,d.view_count?d.view_count.toLocaleString()+' views':null].filter(Boolean).join(' · ');
    bg.innerHTML=`<div class="modal" style="max-width:480px">
      <div class="modal-hdr"><h2>Confirm download</h2></div>
      <div class="modal-body" style="align-items:flex-start;gap:12px">
        ${d.thumbnail?`<img class="preview-thumb" src="${esc(d.thumbnail)}" onerror="this.remove()">`:''}
        <div>
          <div style="font-weight:600;margin-bottom:6px">${esc(d.title||'Unknown')}</div>
          ${d.channel?`<div style="font-size:12px;color:var(--muted);margin-bottom:4px">${esc(d.channel)}</div>`:''}
          <div style="font-size:12px;color:var(--muted)">${esc(info)}</div>
        </div>
      </div>
      <div style="display:flex;gap:8px;justify-content:flex-end">
        <button onclick="this.closest('.modal-bg').remove();setStatus('')">Cancel</button>
        <button class="primary" onclick="confirmDownload('${esc(url)}',this)">⬇ Download</button>
      </div>
    </div>`;
    setStatus('');
  }catch(e){
    bg.innerHTML=`<div class="modal" style="max-width:400px">
      <div class="modal-hdr"><h2>Preview failed</h2></div>
      <div style="color:#f87171;font-size:13px;margin-bottom:8px">${esc(e.message)}</div>
      <div style="display:flex;gap:8px;justify-content:flex-end">
        <button onclick="this.closest('.modal-bg').remove()">Cancel</button>
        <button class="primary" onclick="confirmDownload('${esc(url)}',this)">Download anyway</button>
      </div>
    </div>`;
    setStatus('');
  }
}
function fullScan(){return document.getElementById('dl-full-scan')?.checked||false}
async function confirmDownload(url,btn){
  if(btn)btn.closest('.modal-bg').remove();
  try{
    await api('/api/download',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({url,full_scan:fullScan()})});
    document.getElementById('dl-url').value='';setStatus('Download queued…')
  }catch(e){setStatus('Error: '+e.message)}
}
async function downloadChannel(url){try{await api('/api/download',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({url,full_scan:fullScan()})});setStatus('Checking for new videos…')}catch(e){setStatus('Error: '+e.message)}}
function checkUrlForChannel(name){
  // Mirror check_url_for_folder in downloader.rs: UC+22 chars = channel ID, else handle
  return(/^UC.{22}$/.test(name))
    ?'https://www.youtube.com/channel/'+name
    :'https://www.youtube.com/@'+name;
}
async function downloadChannelByIdx(i){await downloadChannel(checkUrlForChannel(library[i].name))}

/* ── Rescan ─────────────────────────────────────────────────────── */
async function rescan(){try{await api('/api/rescan',{method:'POST'});await loadLibrary()}catch(e){setStatus('Error: '+e.message)}}

/* ── Find video ─────────────────────────────────────────────────── */
function findVideo(id){for(const ch of library)for(const v of[...ch.videos,...ch.playlists.flatMap(p=>p.videos)])if(v.id===id)return v;return null}

/* ── Player ─────────────────────────────────────────────────────── */
function playVideo(id){
  const v=findVideo(id);if(!v||!v.video_url)return;
  currentPlayingId=id;
  const bg=document.createElement('div');bg.className='modal-bg';
  bg.onclick=e=>{if(e.target===bg)closeModal(bg)};
  const tracks=(v.subtitles||[]).map((s,i)=>`<track kind="subtitles" src="${s.url}" srclang="${esc(s.lang)}" label="${esc(s.label)}"${i===0?' default':''}>`).join('');
  const chapPane=v.has_chapters?`<div class="chapters-pane" id="chapters-pane"><h4>Chapters</h4><div id="chapters-list"><em style="padding:10px;display:block;color:var(--muted)">Loading…</em></div></div>`:'';
  bg.innerHTML=`<div class="modal">
    <div class="modal-hdr"><h2>${esc(v.title)}</h2><button onclick="closeModal(this.closest('.modal-bg'))">✕ Close</button></div>
    <div class="modal-body"><video id="player-video" src="${v.video_url}" controls autoplay crossorigin="anonymous">${tracks}</video>${chapPane}</div>
  </div>`;
  document.body.appendChild(bg);
  const vid=bg.querySelector('#player-video');
  if(v.resume_pos&&v.resume_pos>5)vid.addEventListener('loadedmetadata',()=>{vid.currentTime=v.resume_pos},{once:true});
  if(saveTimer)clearInterval(saveTimer);
  saveTimer=setInterval(()=>savePosition(vid),5000);
  vid.addEventListener('pause',()=>savePosition(vid));
  if(v.has_chapters)loadChapters(id);
}

async function savePosition(vid){
  if(!vid||vid.readyState<1||!currentPlayingId)return;
  const pos=vid.currentTime,dur=vid.duration;
  if(!pos||!dur)return;
  const nearEnd=dur>0&&pos>dur*0.95;
  try{
    await fetch('/api/resume/'+encodeURIComponent(currentPlayingId),{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({position:nearEnd?0:pos})});
    if(nearEnd){const v=findVideo(currentPlayingId);if(v&&!v.watched)await api('/api/watched/'+encodeURIComponent(currentPlayingId),{method:'POST'})}
  }catch{}
}

function closeModal(el){
  if(saveTimer){clearInterval(saveTimer);saveTimer=null}
  const vid=el.querySelector('video');
  if(vid){savePosition(vid);vid.pause();vid.src=''}
  currentPlayingId=null;
  el.remove();
  loadLibrary();
}

async function loadChapters(id){
  try{
    const chapters=((await(await fetch('/api/chapters/'+encodeURIComponent(id))).json()).chapters)||[];
    const list=document.getElementById('chapters-list');if(!list)return;
    if(!chapters.length){list.innerHTML='<em style="padding:10px;display:block;color:var(--muted)">No chapters</em>';return}
    list.innerHTML=chapters.map(c=>`<div class="chapter" onclick="jumpToChapter(${c.start})"><div class="ch-time">${fmtDur(c.start)}</div><div class="ch-title">${esc(c.title)}</div></div>`).join('');
    const vid=document.getElementById('player-video');
    if(vid)vid.addEventListener('timeupdate',()=>{
      const t=vid.currentTime;let ai=-1;
      for(let i=0;i<chapters.length;i++){if(t>=chapters[i].start)ai=i;else break}
      list.querySelectorAll('.chapter').forEach((el,i)=>{el.classList.toggle('active',i===ai)});
      const a=list.querySelector('.chapter.active');if(a)a.scrollIntoView({block:'nearest'});
    });
  }catch{}
}
function jumpToChapter(s){const v=document.getElementById('player-video');if(v){v.currentTime=s;v.play()}}

/* ── Select / Details ───────────────────────────────────────────── */
function selectVideo(id){selectedId=selectedId===id?null:id;renderGrid();renderDetails()}
function closeDetails(){selectedId=null;renderGrid();renderDetails()}

function renderDetails(){
  const el=document.getElementById('details');
  if(!selectedId){el.style.display='none';return}
  const v=findVideo(selectedId);if(!v){el.style.display='none';return}
  el.style.display='block';
  const meta=[
    v.duration_secs!=null?fmtDur(v.duration_secs):null,
    v.file_size!=null?fmtSize(v.file_size):null,
    v.resume_pos&&v.duration_secs?'Resume at '+fmtDur(v.resume_pos):null,
    v.watched?'watched':'unwatched',
    v.has_live_chat?'💬 live chat':null,
    !v.has_video?'no file':null,
  ].filter(Boolean);
  const playBtn=v.has_video&&v.video_url?`<button class="primary" onclick="playVideo('${v.id}')">▶ Play</button>`:'';
  el.innerHTML=`<div class="det-head">
    <h3>${esc(v.title)}</h3>
    <div class="det-actions">
      ${playBtn}
      <button onclick="toggleWatched('${v.id}')">${v.watched?'✓ Watched':'○ Unwatched'}</button>
      <button onclick="showMetadata('${v.id}')">📋 Metadata</button>
      <button onclick="closeDetails()">✕</button>
    </div>
  </div>
  <div class="det-meta">${meta.map(esc).join('<span style="color:var(--border)"> | </span>')}</div>
  <div class="det-desc" id="det-desc"><em style="color:var(--muted)">Loading…</em></div>`;
  fetchDescription(selectedId);
}

async function fetchDescription(id){
  try{const t=await(await fetch('/api/description/'+encodeURIComponent(id))).text();const el=document.getElementById('det-desc');if(el)el.textContent=t.trim()||'(no description)'}
  catch{const el=document.getElementById('det-desc');if(el)el.textContent='(could not load description)'}
}

/* ── Metadata viewer ────────────────────────────────────────────── */
async function showMetadata(id){
  let data;try{const r=await fetch('/api/metadata/'+encodeURIComponent(id));if(!r.ok)throw new Error(await r.text());data=await r.json()}catch(e){setStatus('Error: '+e.message);return}
  const bg=document.createElement('div');bg.className='modal-bg';bg.onclick=e=>{if(e.target===bg)bg.remove()};
  const fields=[
    ['Title',data.title],['ID',data.id],['Channel',data.channel||data.uploader],
    ['Channel URL',data.channel_url?`<a href="${esc(data.channel_url)}" target="_blank">${esc(data.channel_url)}</a>`:null],
    ['Original URL',data.webpage_url?`<a href="${esc(data.webpage_url)}" target="_blank">${esc(data.webpage_url)}</a>`:null],
    ['Upload date',data.upload_date?`${data.upload_date.slice(0,4)}-${data.upload_date.slice(4,6)}-${data.upload_date.slice(6,8)}`:null],
    ['Duration',data.duration!=null?fmtDur(data.duration):null],
    ['Views',data.view_count!=null?data.view_count.toLocaleString():null],
    ['Likes',data.like_count!=null?data.like_count.toLocaleString():null],
    ['Comments',data.comment_count!=null?data.comment_count.toLocaleString():null],
    ['Format',data.format],['Resolution',data.resolution||(data.width&&data.height?`${data.width}x${data.height}`:null)],
    ['FPS',data.fps],['Video codec',data.vcodec],['Audio codec',data.acodec],
    ['Filesize',data.filesize_approx?fmtSize(data.filesize_approx):null],
    ['Categories',Array.isArray(data.categories)?data.categories.join(', '):null],
    ['Tags',Array.isArray(data.tags)?data.tags.slice(0,30).join(', '):null],
    ['Age limit',data.age_limit],['Live status',data.live_status],['Availability',data.availability],
  ].filter(([,v])=>v!=null&&v!=='');
  const sum=`<dl class="meta-grid">${fields.map(([k,v])=>`<dt>${esc(k)}</dt><dd>${typeof v==='string'&&v.startsWith('<a')?v:esc(v)}</dd>`).join('')}</dl>`;
  const raw=`<pre class="meta-raw">${esc(JSON.stringify(data,null,2))}</pre>`;
  bg.innerHTML=`<div class="modal" style="max-width:700px">
    <div class="modal-hdr"><h2>Metadata</h2><button onclick="this.closest('.modal-bg').remove()">✕</button></div>
    <div class="meta-tabs">
      <div class="meta-tab active" onclick="switchMetaTab(this,'summary')">Summary</div>
      <div class="meta-tab" onclick="switchMetaTab(this,'raw')">Raw JSON</div>
    </div>
    <div id="meta-summary" style="overflow:auto;max-height:65vh">${sum}</div>
    <div id="meta-raw" style="display:none">${raw}</div>
  </div>`;
  document.body.appendChild(bg);
}
function switchMetaTab(tab,which){tab.parentElement.querySelectorAll('.meta-tab').forEach(t=>t.classList.remove('active'));tab.classList.add('active');document.getElementById('meta-summary').style.display=which==='summary'?'':'none';document.getElementById('meta-raw').style.display=which==='raw'?'':'none'}

/* ── Settings ───────────────────────────────────────────────────── */
const THEMES=[['dark','Dark'],['light','Light'],['dracula','Dracula'],['trans','Trans'],['emo-nocturnal','Emo: Nocturnal'],['emo-coffin','Emo: Coffin'],['emo-scene-queen','Emo: Scene Queen'],['solarized','Solarized'],['nord','Nord'],['amoled','AMOLED']];
function applyTheme(t){document.body.className=t==='dark'?'':'theme-'+t;localStorage.setItem('theme',t)}

async function logout(){try{await fetch('/api/logout',{method:'POST'});location.reload()}catch{}}

async function openSettings(){
  let cur={transcode:false,source_url:null,current_bind:null,available_binds:[],download_password_required:false};try{cur=await(await api('/api/settings')).json()}catch{}
  const savedTheme=localStorage.getItem('theme')||'dark';
  const srcRow=cur.source_url
    ?`<div class="settings-hint" style="margin-top:8px">Source code: <a href="${esc(cur.source_url)}" target="_blank">${esc(cur.source_url)}</a> (AGPL-3.0)</div>`
    :`<div class="settings-hint" style="margin-top:8px;color:var(--muted)">AGPL-3.0 — set <code>web.source_url</code> in config.toml to link source code</div>`;
  const bindRows=cur.available_binds?.length?`<div class="settings-row" style="flex-direction:column;align-items:flex-start;gap:6px">
      <label>Binding</label>
      <div style="font-size:12px;color:var(--muted);margin-bottom:4px">Current: <code style="background:var(--bg);padding:2px 4px;border-radius:2px">${esc(cur.current_bind||'unknown')}</code></div>
      <select id="cf-bind" style="background:var(--card);color:var(--text);border:1px solid var(--border);padding:4px 8px;border-radius:4px;width:100%">
        ${cur.available_binds.map(b=>`<option value="${esc(b.id)}">${esc(b.label)}</option>`).join('')}
      </select>
      <div class="settings-hint">Change requires restart. Access from: tailscale, LAN, or all interfaces.</div>
    </div>`:''
  const logoutBtn=cur.download_password_required?`<button onclick="logout()">Log out</button>`:'';
  let ck={exists:false,cookies:0};try{ck=await(await api('/api/cookies')).json()}catch{}
  const cookiesStatus=ck.exists?`${ck.cookies} cookie(s) loaded`:'no cookies.txt';
  const bg=document.createElement('div');bg.className='modal-bg';bg.onclick=e=>{if(e.target===bg)bg.remove()};
  bg.innerHTML=`<div class="modal" style="max-width:420px">
    <div class="modal-hdr"><h2>Settings</h2></div>
    <div class="settings-row">
      <label for="cf-transcode">Transcode videos (mp4/H.264)</label>
      <input type="checkbox" id="cf-transcode" ${cur.transcode?'checked':''}>
    </div>
    <div class="settings-hint" style="margin-bottom:10px">Requires ffmpeg. Lets Chrome play MKV files. Seeking disabled while transcoding.</div>
    <div class="settings-row">
      <label>Theme</label>
      <select id="cf-theme" onchange="applyTheme(this.value)" style="background:var(--card);color:var(--text);border:1px solid var(--border);padding:4px 8px;border-radius:4px">
        ${THEMES.map(([id,label])=>`<option value="${id}"${savedTheme===id?' selected':''}>${label}</option>`).join('')}
      </select>
    </div>
    <div class="settings-row">
      <label for="cf-download-pwd">Require password to access UI</label>
      <input type="checkbox" id="cf-download-pwd" ${cur.download_password_required?'checked':''} onchange="document.getElementById('cf-pwd-input').style.display=this.checked?'flex':'none'">
    </div>
    <div id="cf-pwd-input" style="display:${cur.download_password_required?'flex':'none'};flex-direction:column;gap:4px;margin-bottom:10px">
      <input type="password" id="cf-download-password" placeholder="New password (leave empty to disable)" style="background:var(--bg);color:var(--text);border:1px solid var(--border);padding:6px 8px;border-radius:4px">
      <div class="settings-hint">Gates the whole UI and all API access. Leave empty to disable on save; changing it logs out other sessions.</div>
    </div>
    ${bindRows}
    <div class="settings-row" style="flex-direction:column;align-items:flex-start;gap:6px">
      <label>Cookies (cookies.txt)</label>
      <div class="settings-hint" id="cookies-status">${cookiesStatus}</div>
      <input type="file" id="cf-cookies-file" accept=".txt,text/plain" onchange="loadCookieFile(this)" style="font-size:11px;color:var(--muted)">
      <textarea id="cf-cookies" placeholder="…or paste Netscape-format cookies.txt here" style="width:100%;height:70px;background:var(--bg);color:var(--text);border:1px solid var(--border);border-radius:4px;padding:6px 8px;font-family:monospace;font-size:11px;resize:vertical"></textarea>
      <div style="display:flex;gap:6px">
        <button onclick="saveCookies(this)">Update cookies</button>
        <button onclick="clearCookies(this)" style="color:var(--muted)">Clear</button>
      </div>
      <div class="settings-hint">Choose a file or paste (e.g. from "Get cookies.txt LOCALLY"). Refresh when downloads start hitting captchas.</div>
    </div>
    ${srcRow}
    <div style="display:flex;gap:8px;justify-content:flex-end;margin-top:12px">
      ${logoutBtn}
      <button onclick="this.closest('.modal-bg').remove()">Cancel</button>
      <button class="primary" onclick="saveSettings(this)">Save</button>
    </div>
  </div>`;
  document.body.appendChild(bg);
}

function loadCookieFile(input){
  const f=input.files&&input.files[0];if(!f)return;
  const r=new FileReader();
  r.onload=()=>{document.getElementById('cf-cookies').value=r.result;setStatus('Loaded '+f.name+' — click Update cookies to save')};
  r.onerror=()=>setStatus('Could not read file');
  r.readAsText(f);
}

async function saveCookies(btn){
  const t=document.getElementById('cf-cookies').value;
  if(!t.trim()){setStatus('Paste cookies first');return}
  btn.disabled=true;
  try{
    const r=await fetch('/api/cookies',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({cookies:t})});
    if(!r.ok)throw new Error(await r.text());
    const d=await r.json();
    document.getElementById('cf-cookies').value='';
    document.getElementById('cookies-status').textContent=d.cookies+' cookie(s) loaded';
    setStatus('Cookies updated ('+d.cookies+' entries)');
  }catch(e){setStatus('Cookies error: '+e.message)}finally{btn.disabled=false}
}
async function clearCookies(btn){
  if(!confirm('Remove cookies.txt? Downloads requiring login will fail until you add new cookies.'))return;
  btn.disabled=true;
  try{
    await api('/api/cookies',{method:'DELETE'});
    document.getElementById('cookies-status').textContent='no cookies.txt';
    setStatus('Cookies cleared');
  }catch(e){setStatus('Error: '+e.message)}finally{btn.disabled=false}
}

async function saveSettings(btn){
  const transcode=document.getElementById('cf-transcode').checked;
  const bindMode=document.getElementById('cf-bind')?.value;
  const pwdCheckbox=document.getElementById('cf-download-pwd');
  const pwdInput=document.getElementById('cf-download-password');
  const payload={transcode};
  if(bindMode)payload.bind_mode=bindMode;
  if(pwdCheckbox&&pwdCheckbox.checked){
    payload.new_download_password=pwdInput?.value||'';
  }
  const settingPwd=pwdCheckbox&&pwdCheckbox.checked&&pwdInput?.value;
  try{
    await api('/api/settings',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(payload)});
    btn.closest('.modal-bg').remove();
    if(settingPwd){setStatus('Password set — signing in again…');location.reload();return}
    let msg='Saved.';
    if(bindMode)msg='Settings saved. Restart required for binding change.';
    if(pwdCheckbox&&pwdCheckbox.checked&&!pwdInput?.value)msg='Password disabled. '+msg;
    setStatus(msg);await loadLibrary();
  }catch(e){setStatus('Error: '+e.message)}
}

/* ── Maintenance (library health) ───────────────────────────────── */
async function openMaintenance(){
  const bg=document.createElement('div');bg.className='modal-bg';bg.onclick=e=>{if(e.target===bg)bg.remove()};
  bg.innerHTML=`<div class="modal" style="max-width:760px;width:100%">
    <div class="modal-hdr"><h2>Library health</h2><button onclick="this.closest('.modal-bg').remove()">✕</button></div>
    <div id="maint-body" style="overflow:auto;max-height:75vh"><em style="color:var(--muted)">Scanning…</em></div>
  </div>`;
  document.body.appendChild(bg);
  try{
    const r=await(await api('/api/maintenance/scan')).json();
    renderMaintenance(r);
  }catch(e){document.getElementById('maint-body').innerHTML=`<div style="color:#f87171">Scan failed: ${esc(e.message)}</div>`}
}

function renderMaintenance(r){
  const body=document.getElementById('maint-body');if(!body)return;
  const dups=r.duplicates||[],miss=r.missing||[];
  let h='';
  h+=`<h3 style="font-size:13px;margin:4px 0 8px">Duplicates (${dups.length})</h3>`;
  if(!dups.length){h+='<div style="color:var(--muted);font-size:12px;margin-bottom:12px">No duplicate video IDs found.</div>'}
  else{
    for(const g of dups){
      h+=`<div style="border:1px solid var(--border);border-radius:6px;padding:8px;margin-bottom:8px">
        <div style="font-weight:600;font-size:12px;margin-bottom:6px">${esc(g.title)} <span style="color:var(--muted)">[${esc(g.id)}]</span></div>`;
      g.copies.forEach((c,i)=>{
        const tag=c.recommended_keep?'<span style="color:#4ade80">keep</span>':'<span style="color:#f87171">remove</span>';
        const size=c.file_size?fmtSize(c.file_size):'no video';
        h+=`<label style="display:flex;align-items:center;gap:8px;font-size:12px;padding:3px 0">
          <input type="checkbox" class="dup-chk" data-files='${esc(JSON.stringify(c.files))}' ${c.recommended_keep?'':'checked'}>
          <span style="flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(c.location||'(unknown)')} · ${size} · ${c.files.length} files</span>
          ${tag}
        </label>`;
      });
      h+=`</div>`;
    }
    h+=`<button class="primary" onclick="removeDuplicates(this)">🗑 Remove checked copies</button>`;
  }
  h+=`<h3 style="font-size:13px;margin:16px 0 8px">Missing assets (${miss.length})</h3>`;
  if(!miss.length){h+='<div style="color:var(--muted);font-size:12px">Every video has its thumbnail, metadata, and description.</div>'}
  else{
    if(miss.length>1)h+=`<button onclick="repairAll(this)" style="margin-bottom:8px">⬇ Fetch all missing (${miss.length})</button>`;
    for(const m of miss){
      const need=[m.missing_thumbnail?'thumbnail':null,m.missing_info?'metadata':null,m.missing_description?'description':null].filter(Boolean).join(', ');
      h+=`<div style="display:flex;align-items:center;gap:8px;font-size:12px;padding:4px 0;border-bottom:1px solid var(--border)">
        <span style="flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(m.title)} <span style="color:var(--muted)">— missing ${need}</span></span>
        <button onclick="repairVideo('${esc(m.id)}',this)">⬇ Fetch</button>
      </div>`;
    }
  }
  body.innerHTML=h;
}

async function removeDuplicates(btn){
  const chks=[...document.querySelectorAll('.dup-chk:checked')];
  let paths=[];
  for(const c of chks){try{paths=paths.concat(JSON.parse(c.dataset.files))}catch{}}
  if(!paths.length){setStatus('Nothing selected.');return}
  if(!confirm(`Delete ${paths.length} file(s)? This cannot be undone.`))return;
  btn.disabled=true;
  try{
    const r=await(await api('/api/maintenance/remove',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({paths})})).json();
    setStatus(`Removed ${r.removed} file(s)`+(r.errors&&r.errors.length?`, ${r.errors.length} error(s)`:''));
    await loadLibrary();
    const fresh=await(await api('/api/maintenance/scan')).json();renderMaintenance(fresh);
  }catch(e){setStatus('Error: '+e.message)}finally{btn.disabled=false}
}

async function repairVideo(id,btn){
  if(btn){btn.disabled=true;btn.textContent='⏳ Queued'}
  try{await api('/api/maintenance/repair/'+encodeURIComponent(id),{method:'POST'});setStatus('Repair queued — see Downloads')}
  catch(e){setStatus('Error: '+e.message);if(btn){btn.disabled=false;btn.textContent='⬇ Fetch'}}
}
async function repairAll(btn){
  btn.disabled=true;
  const buttons=[...document.querySelectorAll('#maint-body button')].filter(b=>b.textContent.includes('Fetch')&&b!==btn);
  for(const b of buttons){const id=b.getAttribute('onclick')?.match(/repairVideo\('([^']+)'/)?.[1];if(id)await repairVideo(id,b)}
  setStatus('All repairs queued — see Downloads');
}

/* ── Jobs ───────────────────────────────────────────────────────── */
function renderJobs(jobs){
  const el=document.getElementById('jobs');
  if(!jobs.length){el.innerHTML='';return}
  const fin=jobs.some(j=>j.state!=='running');
  const hdr=fin?`<div style="padding:4px 14px;display:flex;justify-content:flex-end;border-bottom:1px solid var(--border)"><button onclick="clearFinishedJobs()" style="font-size:11px;padding:2px 8px">✕ Clear finished</button></div>`:'';
  el.innerHTML=hdr+jobs.map((j,i)=>{
    const dismiss=j.state!=='running'?`<button onclick="removeJob(${i})" style="font-size:11px;padding:1px 6px">✕</button>`:'';
    return `<div class="job">
      <span class="badge ${j.state}">${j.state}</span>
      <span style="flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(j.label)} — ${esc(j.url)}</span>
      ${j.state==='running'?`<progress value="${j.progress}" max="1"></progress>`:''}
      <span style="font-size:11px;color:var(--muted);max-width:160px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(j.last_line)}</span>
      ${dismiss}
    </div>`;
  }).join('');
}
async function clearFinishedJobs(){try{await api('/api/jobs/clear',{method:'POST'});await pollProgress()}catch(e){setStatus('Error: '+e.message)}}
async function removeJob(idx){try{await api('/api/jobs/'+idx,{method:'DELETE'});await pollProgress()}catch(e){setStatus('Error: '+e.message)}}

let wasRunning=false;
async function pollProgress(){
  try{const{jobs}=await(await api('/api/progress')).json();renderJobs(jobs);const run=jobs.some(j=>j.state==='running');if(wasRunning&&!run)await loadLibrary();wasRunning=run}catch{}
}
setInterval(pollProgress,600);

/* ── Utilities ──────────────────────────────────────────────────── */
function fmtDur(s){s=Math.floor(s);const h=Math.floor(s/3600),m=Math.floor((s%3600)/60),sec=s%60;return h?`${h}:${p(m)}:${p(sec)}`:`${m}:${p(sec)}`}
function fmtSize(b){if(b>=1073741824)return(b/1073741824).toFixed(1)+' GB';if(b>=1048576)return Math.round(b/1048576)+' MB';return Math.round(b/1024)+' KB'}
function p(n){return String(n).padStart(2,'0')}
function esc(s){return String(s??'').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;')}

/* ── AGPL §13 source notice ─────────────────────────────────────── */
// On load, fetch the source URL from the server and render it in the footer.
// This satisfies the AGPL §13 requirement that network users are offered
// access to the Corresponding Source of the software they interact with.
async function loadSourceNotice(){
  try{
    const s=await(await fetch('/api/settings')).json();
    const el=document.getElementById('agpl-notice');
    if(!el)return;
    if(s.source_url){
      el.innerHTML=`Licensed under <a href="https://www.gnu.org/licenses/agpl-3.0.html" target="_blank">AGPL-3.0</a> &mdash; <a href="${esc(s.source_url)}" target="_blank">Source code</a>`;
    } else {
      el.textContent='Licensed under AGPL-3.0 — set web.source_url in config.toml to link source code';
    }
  }catch{}
}

/* ── Init ───────────────────────────────────────────────────────── */
applyTheme(localStorage.getItem('theme')||'dark');
loadLibrary();
loadSourceNotice();
</script>
</body>
</html>"#;
