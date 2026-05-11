//! Web interface — run with `--web [PORT]` instead of the GUI.
//!
//! Serves a browser-based UI on http://localhost:PORT that mirrors the
//! desktop app's core features: browse library, start downloads, track
//! progress, toggle watched.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Sse},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;

use crate::config::Config;
use crate::database::Database;
use crate::downloader::{detect_url_kind, Downloader, JobState};
use crate::library;

// ── Shared state ─────────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct JobSnapshot {
    pub label: String,
    pub url: String,
    pub state: &'static str,
    pub progress: f32,
    pub last_line: String,
}

pub struct WebState {
    pub library: Mutex<Vec<library::Channel>>,
    pub downloader: Mutex<Downloader>,
    pub watched: Mutex<HashSet<String>>,
    pub db_path: PathBuf,
    pub channels_root: PathBuf,
    pub browser: String,
    /// Broadcast channel for SSE progress events
    pub progress_tx: broadcast::Sender<String>,
}

impl WebState {
    pub fn job_snapshots(dl: &Downloader) -> Vec<JobSnapshot> {
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

#[derive(Serialize)]
struct LibraryResponse {
    channels: Vec<ChannelInfo>,
}

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

#[derive(Serialize)]
struct PlaylistInfo {
    name: String,
    videos: Vec<VideoInfo>,
}

#[derive(Serialize)]
struct VideoInfo {
    id: String,
    title: String,
    duration_secs: Option<f64>,
    file_size: Option<u64>,
    has_video: bool,
    has_live_chat: bool,
    watched: bool,
}

#[derive(Deserialize)]
struct StartDownloadRequest {
    url: String,
}

#[derive(Serialize)]
struct ProgressResponse {
    jobs: Vec<JobSnapshot>,
}

// ── Route handlers ────────────────────────────────────────────────────────────

async fn get_index() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        HTML_UI,
    )
}

async fn get_library(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let lib = state.library.lock().unwrap();
    let watched = state.watched.lock().unwrap();

    let channels = lib
        .iter()
        .map(|ch| {
            let size_bytes: u64 = ch
                .videos
                .iter()
                .chain(ch.playlists.iter().flat_map(|p| p.videos.iter()))
                .filter_map(|v| v.file_size)
                .sum();

            let to_info = |v: &library::Video| VideoInfo {
                id: v.id.clone(),
                title: v.title.clone(),
                duration_secs: v.duration_secs,
                file_size: v.file_size,
                has_video: v.video_path.is_some(),
                has_live_chat: v.has_live_chat,
                watched: watched.contains(&v.id),
            };

            ChannelInfo {
                name: ch.name.clone(),
                total_videos: ch.total_videos(),
                size_bytes,
                subscriber_count: ch.meta.as_ref().and_then(|m| m.subscriber_count),
                uploader: ch.meta.as_ref().and_then(|m| m.uploader.clone()),
                channel_url: ch.meta.as_ref().and_then(|m| m.channel_url.clone()),
                playlists: ch
                    .playlists
                    .iter()
                    .map(|p| PlaylistInfo {
                        name: p.name.clone(),
                        videos: p.videos.iter().map(to_info).collect(),
                    })
                    .collect(),
                videos: ch.videos.iter().map(to_info).collect(),
            }
        })
        .collect();

    Json(LibraryResponse { channels })
}

async fn post_download(
    State(state): State<Arc<WebState>>,
    Json(body): Json<StartDownloadRequest>,
) -> impl IntoResponse {
    let url = body.url.trim().to_string();
    if url.is_empty() {
        return (StatusCode::BAD_REQUEST, "empty URL").into_response();
    }
    let kind = detect_url_kind(&url);
    {
        let mut dl = state.downloader.lock().unwrap();
        dl.start(url, &kind);
    }
    (StatusCode::ACCEPTED, "ok").into_response()
}

async fn get_progress(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let dl = state.downloader.lock().unwrap();
    Json(ProgressResponse { jobs: WebState::job_snapshots(&dl) })
}

async fn post_watched(
    State(state): State<Arc<WebState>>,
    Path(video_id): Path<String>,
) -> impl IntoResponse {
    let db = match Database::open(&state.db_path) {
        Ok(d) => d,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let mut watched = state.watched.lock().unwrap();
    let now_watched = !watched.contains(&video_id);
    if let Err(e) = db.set_watched(&video_id, now_watched) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    if now_watched {
        watched.insert(video_id);
    } else {
        watched.remove(&video_id);
    }
    (StatusCode::OK, if now_watched { "watched" } else { "unwatched" }).into_response()
}

async fn get_events(State(state): State<Arc<WebState>>) -> Sse<impl tokio_stream::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>> {
    let rx = state.progress_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| {
        msg.ok().map(|data| {
            Ok(axum::response::sse::Event::default().data(data))
        })
    });
    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

async fn post_rescan(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let root = state.channels_root.clone();
    let new_lib = library::scan_channels(&root);
    *state.library.lock().unwrap() = new_lib;
    (StatusCode::OK, "rescanned")
}

// ── Server entry point ────────────────────────────────────────────────────────

pub fn run(config: Config) -> ! {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async move {
        serve(config).await;
    });
    unreachable!()
}

async fn serve(config: Config) {
    let channels_root = config.backup.directory.clone();
    let db_path = channels_root.join("yt-offline.db");
    let library = library::scan_channels(&channels_root);

    let db = Database::open(&db_path).expect("web: open db");
    let watched = db.get_watched().unwrap_or_default();

    let browser = config.player.browser.clone();
    let downloader = Downloader::new(channels_root.clone(), browser.clone());

    let (progress_tx, _) = broadcast::channel::<String>(64);

    let state = Arc::new(WebState {
        library: Mutex::new(library),
        downloader: Mutex::new(downloader),
        watched: Mutex::new(watched),
        db_path,
        channels_root,
        browser,
        progress_tx: progress_tx.clone(),
    });

    // Background task: poll downloader and broadcast SSE events
    let poll_state = Arc::clone(&state);
    tokio::spawn(async move {
        loop {
            {
                let mut dl = poll_state.downloader.lock().unwrap();
                dl.poll();
                let snap = WebState::job_snapshots(&dl);
                drop(dl);
                if let Ok(json) = serde_json::to_string(&snap) {
                    let _ = poll_state.progress_tx.send(json);
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    });

    let app = Router::new()
        .route("/", get(get_index))
        .route("/api/library", get(get_library))
        .route("/api/download", post(post_download))
        .route("/api/progress", get(get_progress))
        .route("/api/events", get(get_events))
        .route("/api/watched/:id", post(post_watched))
        .route("/api/rescan", post(post_rescan))
        .with_state(state);

    let port = config.web.port;
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    println!("yt-offline web UI: http://localhost:{port}");
    axum::serve(listener, app).await.expect("serve");
}

// ── Embedded HTML/JS UI ───────────────────────────────────────────────────────

const HTML_UI: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>yt-offline</title>
<style>
  :root {
    --bg: #1a1a2e; --panel: #16213e; --card: #0f3460;
    --accent: #e94560; --text: #eee; --muted: #aaa; --border: #334;
  }
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body { background: var(--bg); color: var(--text); font: 14px/1.5 system-ui, sans-serif; display: flex; flex-direction: column; height: 100vh; }
  header { background: var(--panel); padding: 10px 16px; display: flex; gap: 12px; align-items: center; border-bottom: 1px solid var(--border); }
  header h1 { font-size: 1.1em; }
  header input { flex: 1; background: var(--bg); border: 1px solid var(--border); color: var(--text); padding: 6px 10px; border-radius: 4px; }
  button { background: var(--accent); color: #fff; border: none; padding: 6px 14px; border-radius: 4px; cursor: pointer; font-size: 13px; }
  button:hover { opacity: 0.85; }
  button.muted { background: var(--card); }
  main { display: flex; flex: 1; overflow: hidden; }
  aside { width: 220px; background: var(--panel); border-right: 1px solid var(--border); overflow-y: auto; padding: 8px 0; flex-shrink: 0; }
  aside h3 { padding: 6px 12px; font-size: 0.75em; text-transform: uppercase; color: var(--muted); letter-spacing: 0.08em; }
  .ch-item { padding: 6px 12px; cursor: pointer; font-size: 13px; border-left: 3px solid transparent; }
  .ch-item:hover { background: var(--card); }
  .ch-item.active { border-left-color: var(--accent); background: var(--card); }
  .ch-sub { padding: 4px 12px 4px 24px; font-size: 12px; color: var(--muted); cursor: pointer; }
  .ch-sub:hover { color: var(--text); }
  section#content { flex: 1; overflow-y: auto; padding: 12px; }
  .video-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 12px; }
  .video-card { background: var(--card); border-radius: 6px; overflow: hidden; cursor: pointer; border: 2px solid transparent; transition: border-color 0.15s; }
  .video-card:hover { border-color: var(--accent); }
  .video-card.watched { opacity: 0.55; }
  .thumb { width: 100%; aspect-ratio: 16/9; background: #222; display: flex; align-items: center; justify-content: center; font-size: 2em; color: #555; }
  .card-body { padding: 8px; }
  .card-title { font-size: 13px; font-weight: 600; margin-bottom: 4px; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .card-meta { font-size: 11px; color: var(--muted); display: flex; gap: 6px; flex-wrap: wrap; }
  .card-actions { display: flex; gap: 6px; margin-top: 6px; }
  .card-actions button { font-size: 11px; padding: 3px 8px; }
  #download-bar { background: var(--panel); border-top: 1px solid var(--border); padding: 10px 16px; display: flex; gap: 8px; align-items: center; }
  #download-bar input { flex: 1; background: var(--bg); border: 1px solid var(--border); color: var(--text); padding: 6px 10px; border-radius: 4px; }
  #jobs { background: var(--panel); border-top: 1px solid var(--border); }
  .job { padding: 6px 16px; display: flex; align-items: center; gap: 10px; font-size: 12px; border-bottom: 1px solid var(--border); }
  .job-state { font-weight: 700; min-width: 50px; }
  .job-state.running { color: #facc15; }
  .job-state.done { color: #4ade80; }
  .job-state.failed { color: #f87171; }
  progress { flex: 1; height: 6px; accent-color: var(--accent); }
  #status { font-size: 12px; color: var(--muted); padding: 0 16px; }
  .badge { background: var(--accent); color: #fff; border-radius: 10px; padding: 1px 7px; font-size: 10px; }
</style>
</head>
<body>
<header>
  <h1>yt-offline</h1>
  <input type="search" id="search" placeholder="Filter by title or ID…" oninput="filterCards()">
  <button class="muted" onclick="rescan()">⟳ Rescan</button>
  <span id="status"></span>
</header>
<main>
  <aside id="sidebar"></aside>
  <section id="content"><div class="video-grid" id="grid"></div></section>
</main>
<div id="jobs"></div>
<div id="download-bar">
  <input type="url" id="dl-url" placeholder="YouTube URL to download…" onkeydown="if(event.key==='Enter')startDownload()">
  <button onclick="startDownload()">⬇ Download</button>
</div>

<script>
let library = [];
let activeChannel = null;
let activePlaylist = null;

async function loadLibrary() {
  const r = await fetch('/api/library');
  library = (await r.json()).channels;
  renderSidebar();
  renderGrid();
}

function renderSidebar() {
  const el = document.getElementById('sidebar');
  let total = library.reduce((s, c) => s + c.total_videos, 0);
  let html = `<h3>Channels</h3>
    <div class="ch-item ${!activeChannel ? 'active' : ''}" onclick="selectChannel(null)">⊞ All (${total})</div>`;
  for (const ch of library) {
    const active = activeChannel === ch.name && !activePlaylist;
    html += `<div class="ch-item ${active ? 'active' : ''}" onclick="selectChannel('${esc(ch.name)}')">
      ${esc(ch.name)} <span style="color:var(--muted);font-size:11px">(${ch.total_videos})</span>
    </div>`;
    if (activeChannel === ch.name && ch.playlists.length) {
      for (const pl of ch.playlists) {
        const ap = activePlaylist === pl.name;
        html += `<div class="ch-sub ${ap ? 'active' : ''}" onclick="selectPlaylist('${esc(ch.name)}','${esc(pl.name)}')">└ ${esc(pl.name)} (${pl.videos.length})</div>`;
      }
    }
  }
  el.innerHTML = html;
}

function selectChannel(name) {
  activeChannel = name; activePlaylist = null;
  renderSidebar(); renderGrid();
}
function selectPlaylist(ch, pl) {
  activeChannel = ch; activePlaylist = pl;
  renderSidebar(); renderGrid();
}

function currentVideos() {
  const q = document.getElementById('search').value.toLowerCase();
  let videos = [];
  for (const ch of library) {
    if (activeChannel && ch.name !== activeChannel) continue;
    const pool = activePlaylist
      ? (ch.playlists.find(p => p.name === activePlaylist)?.videos || [])
      : [...ch.videos, ...ch.playlists.flatMap(p => p.videos)];
    for (const v of pool) {
      if (!q || v.title.toLowerCase().includes(q) || v.id.includes(q))
        videos.push({...v, channel: ch.name});
    }
  }
  return videos;
}

function filterCards() { renderGrid(); }

function renderGrid() {
  const videos = currentVideos();
  document.getElementById('status').textContent = `${videos.length} videos`;
  const grid = document.getElementById('grid');
  grid.innerHTML = videos.map(v => `
    <div class="video-card ${v.watched ? 'watched' : ''}" id="card-${v.id}">
      <div class="thumb">▶</div>
      <div class="card-body">
        <div class="card-title">${esc(v.title)}</div>
        <div class="card-meta">
          <span>${esc(v.channel)}</span>
          ${v.duration_secs ? `<span>${fmtDur(v.duration_secs)}</span>` : ''}
          ${v.file_size ? `<span>${fmtSize(v.file_size)}</span>` : ''}
          ${v.has_live_chat ? '<span>💬</span>' : ''}
          ${!v.has_video ? '<span style="color:#f87171">no file</span>' : ''}
        </div>
        <div class="card-actions">
          <button onclick="toggleWatched('${v.id}')">${v.watched ? '✓ Watched' : '○ Watch'}</button>
        </div>
      </div>
    </div>`).join('');
}

async function toggleWatched(id) {
  await fetch(`/api/watched/${id}`, {method:'POST'});
  await loadLibrary();
}

async function startDownload() {
  const url = document.getElementById('dl-url').value.trim();
  if (!url) return;
  await fetch('/api/download', {method:'POST', headers:{'Content-Type':'application/json'}, body: JSON.stringify({url})});
  document.getElementById('dl-url').value = '';
}

async function rescan() {
  await fetch('/api/rescan', {method:'POST'});
  await loadLibrary();
}

// SSE progress
const es = new EventSource('/api/events');
es.onmessage = e => {
  try {
    const jobs = JSON.parse(e.data);
    renderJobs(jobs);
  } catch {}
};

function renderJobs(jobs) {
  if (!jobs.length) { document.getElementById('jobs').innerHTML = ''; return; }
  document.getElementById('jobs').innerHTML = jobs.map(j => `
    <div class="job">
      <span class="job-state ${j.state}">${j.state}</span>
      <span style="flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(j.label)} — ${esc(j.url)}</span>
      ${j.state==='running' ? `<progress value="${j.progress}" max="1"></progress>` : ''}
    </div>`).join('');
  // Rescan library when all done
  if (jobs.length && jobs.every(j => j.state !== 'running')) loadLibrary();
}

function fmtDur(s) {
  s = Math.floor(s);
  const h = Math.floor(s/3600), m = Math.floor((s%3600)/60), sec = s%60;
  return h ? `${h}:${p(m)}:${p(sec)}` : `${m}:${p(sec)}`;
}
function fmtSize(b) {
  if (b>=1073741824) return (b/1073741824).toFixed(1)+' GB';
  if (b>=1048576) return Math.round(b/1048576)+' MB';
  return Math.round(b/1024)+' KB';
}
function p(n) { return String(n).padStart(2,'0'); }
function esc(s) { return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;'); }

loadLibrary();
</script>
</body>
</html>"#;
