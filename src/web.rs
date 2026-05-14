//! Web interface — run with `--web [PORT]` instead of the GUI.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::database::Database;
use crate::downloader::{detect_url_kind, Downloader, JobState};
use crate::library;

// ── Shared state ──────────────────────────────────────────────────────────────

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

    let to_info = |v: &library::Video, watched: &HashSet<String>| VideoInfo {
        id: v.id.clone(),
        title: v.title.clone(),
        duration_secs: v.duration_secs,
        file_size: v.file_size,
        has_video: v.video_path.is_some(),
        has_live_chat: v.has_live_chat,
        watched: watched.contains(&v.id),
    };

    let channels = lib.iter().map(|ch| {
        let size_bytes: u64 = ch.videos.iter()
            .chain(ch.playlists.iter().flat_map(|p| p.videos.iter()))
            .filter_map(|v| v.file_size)
            .sum();
        ChannelInfo {
            name: ch.name.clone(),
            total_videos: ch.total_videos(),
            size_bytes,
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
    let url = body.url.trim().to_string();
    if url.is_empty() {
        return (StatusCode::BAD_REQUEST, "empty URL").into_response();
    }
    let kind = detect_url_kind(&url);
    state.downloader.lock().unwrap().start(url, &kind);
    (StatusCode::ACCEPTED, "ok").into_response()
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
    if now_watched { watched.insert(video_id); } else { watched.remove(&video_id); }
    (StatusCode::OK, if now_watched { "watched" } else { "unwatched" }).into_response()
}

async fn post_rescan(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let new_lib = library::scan_channels(&state.channels_root);
    // Refresh watched from DB after rescan
    if let Ok(db) = Database::open(&state.db_path) {
        if let Ok(w) = db.get_watched() {
            *state.watched.lock().unwrap() = w;
        }
    }
    *state.library.lock().unwrap() = new_lib;
    (StatusCode::OK, "rescanned")
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn run(config: Config) -> ! {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(serve(config));
    unreachable!()
}

async fn serve(config: Config) {
    let channels_root = config.backup.directory.clone();
    let _ = std::fs::create_dir_all(&channels_root);
    let db_path = channels_root.join("yt-offline.db");

    let library = library::scan_channels(&channels_root);
    let watched = Database::open(&db_path)
        .and_then(|db| db.get_watched())
        .unwrap_or_default();

    let downloader = Downloader::new(channels_root.clone(), config.player.browser.clone());

    let state = Arc::new(WebState {
        library: Mutex::new(library),
        downloader: Mutex::new(downloader),
        watched: Mutex::new(watched),
        db_path,
        channels_root,
    });

    let app = Router::new()
        .route("/", get(get_index))
        .route("/api/library", get(get_library))
        .route("/api/progress", get(get_progress))
        .route("/api/download", post(post_download))
        .route("/api/watched/:id", post(post_watched))
        .route("/api/rescan", post(post_rescan))
        .with_state(state);

    let port = config.web.port;
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .unwrap_or_else(|e| panic!("Cannot bind to port {port}: {e}"));
    println!("yt-offline web UI: http://localhost:{port}");
    axum::serve(listener, app).await.expect("server error");
}

// ── Embedded UI ───────────────────────────────────────────────────────────────

const HTML_UI: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>yt-offline</title>
<style>
  :root {
    --bg:#1a1a2e; --panel:#16213e; --card:#0f3460;
    --accent:#e94560; --text:#eee; --muted:#999; --border:#334;
  }
  *{box-sizing:border-box;margin:0;padding:0}
  body{background:var(--bg);color:var(--text);font:14px/1.5 system-ui,sans-serif;display:flex;flex-direction:column;height:100vh;overflow:hidden}
  header{background:var(--panel);padding:8px 14px;display:flex;gap:10px;align-items:center;border-bottom:1px solid var(--border);flex-shrink:0}
  header h1{font-size:1em;font-weight:700;white-space:nowrap}
  header input{flex:1;background:var(--bg);border:1px solid var(--border);color:var(--text);padding:5px 9px;border-radius:4px;font-size:13px}
  button{background:var(--card);color:var(--text);border:1px solid var(--border);padding:5px 12px;border-radius:4px;cursor:pointer;font-size:12px}
  button:hover{background:var(--accent);border-color:var(--accent);color:#fff}
  button.primary{background:var(--accent);border-color:var(--accent);color:#fff}
  main{display:flex;flex:1;overflow:hidden}
  aside{width:210px;background:var(--panel);border-right:1px solid var(--border);overflow-y:auto;flex-shrink:0;padding:6px 0}
  .sidebar-label{padding:2px 10px;font-size:10px;text-transform:uppercase;color:var(--muted);letter-spacing:.07em;margin-top:6px}
  .ch-item{padding:5px 10px;cursor:pointer;font-size:13px;border-left:3px solid transparent;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  .ch-item:hover{background:var(--card)}
  .ch-item.active{border-left-color:var(--accent);background:rgba(255,255,255,.04)}
  .ch-sub{padding:3px 10px 3px 22px;font-size:12px;color:var(--muted);cursor:pointer;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  .ch-sub:hover{color:var(--text);background:rgba(255,255,255,.03)}
  .ch-sub.active{color:var(--text)}
  section#content{flex:1;overflow-y:auto;padding:10px}
  .toolbar{display:flex;align-items:center;gap:8px;margin-bottom:8px;flex-wrap:wrap}
  .toolbar select{background:var(--card);color:var(--text);border:1px solid var(--border);padding:4px 8px;border-radius:4px;font-size:12px}
  .grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(240px,1fr));gap:10px}
  .card{background:var(--card);border-radius:6px;overflow:hidden;border:2px solid transparent;transition:border-color .15s}
  .card:hover{border-color:var(--accent)}
  .card.watched{opacity:.5}
  .card-title{font-size:13px;font-weight:600;padding:8px 8px 2px;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;min-height:36px}
  .card-meta{font-size:11px;color:var(--muted);padding:0 8px 4px;display:flex;gap:5px;flex-wrap:wrap}
  .card-foot{padding:4px 8px 8px;display:flex;gap:6px}
  .card-foot button{font-size:11px;padding:3px 8px}
  footer{background:var(--panel);border-top:1px solid var(--border);padding:8px 14px;display:flex;gap:8px;align-items:center;flex-shrink:0}
  footer input{flex:1;background:var(--bg);border:1px solid var(--border);color:var(--text);padding:5px 9px;border-radius:4px;font-size:13px}
  #jobs{background:var(--panel);border-top:1px solid var(--border);flex-shrink:0}
  .job{display:flex;align-items:center;gap:8px;padding:5px 14px;font-size:12px;border-bottom:1px solid var(--border)}
  .badge{font-weight:700;min-width:48px}
  .badge.running{color:#facc15}
  .badge.done{color:#4ade80}
  .badge.failed{color:#f87171}
  progress{flex:1;height:5px;accent-color:var(--accent)}
  .empty{text-align:center;color:var(--muted);padding:40px;font-size:13px}
</style>
</head>
<body>
<header>
  <h1>yt-offline</h1>
  <input type="search" id="search" placeholder="Filter…" oninput="renderGrid()">
  <select id="sort" onchange="renderGrid()">
    <option value="title">Title</option>
    <option value="dur-asc">Shortest</option>
    <option value="dur-desc">Longest</option>
    <option value="size-asc">Smallest</option>
    <option value="size-desc">Largest</option>
  </select>
  <button onclick="rescan()">⟳ Rescan</button>
  <span id="status" style="font-size:12px;color:var(--muted)"></span>
</header>
<main>
  <aside id="sidebar"></aside>
  <section id="content">
    <div class="toolbar">
      <label style="font-size:12px;color:var(--muted)">
        <input type="checkbox" id="bulk" onchange="toggleBulk()"> Select
      </label>
      <span id="bulk-actions" style="display:none;gap:6px;display:none">
        <button onclick="bulkWatched(true)">✓ Mark watched</button>
        <button onclick="bulkWatched(false)">○ Unwatch</button>
        <span id="sel-count" style="font-size:12px;color:var(--muted)"></span>
      </span>
    </div>
    <div class="grid" id="grid"></div>
  </section>
</main>
<div id="jobs"></div>
<footer>
  <input type="url" id="dl-url" placeholder="YouTube URL to download…" onkeydown="if(event.key==='Enter')startDownload()">
  <button class="primary" onclick="startDownload()">⬇ Download</button>
</footer>

<script>
'use strict';
let library = [], activeChannel = null, activePlaylist = null;
let bulkMode = false, selected = new Set();

async function api(path, opts) {
  const r = await fetch(path, opts);
  if (!r.ok) throw new Error(await r.text());
  return r;
}

async function loadLibrary() {
  try {
    const r = await api('/api/library');
    library = (await r.json()).channels;
    renderSidebar();
    renderGrid();
  } catch(e) { setStatus('Error: ' + e.message); }
}

function setStatus(s) { document.getElementById('status').textContent = s; }

function renderSidebar() {
  const el = document.getElementById('sidebar');
  const total = library.reduce((s,c)=>s+c.total_videos,0);
  let h = `<div class="sidebar-label">Library</div>`;
  h += sidebar_item(null, null, `⊞ All (${total})`, activeChannel===null);
  for (const ch of library) {
    const active = activeChannel===ch.name && activePlaylist===null;
    const size = ch.size_bytes > 0 ? ' · '+fmtSize(ch.size_bytes) : '';
    h += sidebar_item(ch.name, null, `${esc(ch.name)} (${ch.total_videos}${size})`, active);
    if (activeChannel === ch.name) {
      for (const pl of ch.playlists) {
        h += `<div class="ch-sub${activePlaylist===pl.name?' active':''}"
          onclick="setView(${JSON.stringify(ch.name)},${JSON.stringify(pl.name)})"
          >└ ${esc(pl.name)} (${pl.videos.length})</div>`;
      }
      if (ch.channel_url) {
        h += `<div class="ch-sub" style="color:var(--accent)"
          onclick="downloadChannel(${JSON.stringify(ch.channel_url)})">⬇ Check for new videos</div>`;
      }
    }
  }
  el.innerHTML = h;
}

function sidebar_item(ch, pl, label, active) {
  return `<div class="ch-item${active?' active':''}"
    onclick="setView(${JSON.stringify(ch)},${JSON.stringify(pl)})">${label}</div>`;
}

function setView(ch, pl) {
  activeChannel = ch; activePlaylist = pl;
  selected.clear();
  renderSidebar(); renderGrid();
}

function currentVideos() {
  const q = document.getElementById('search').value.toLowerCase();
  const sort = document.getElementById('sort').value;
  let vids = [];
  for (const ch of library) {
    if (activeChannel && ch.name !== activeChannel) continue;
    const pool = activePlaylist
      ? (ch.playlists.find(p=>p.name===activePlaylist)?.videos || [])
      : [...ch.videos, ...ch.playlists.flatMap(p=>p.videos)];
    for (const v of pool) {
      if (!q || v.title.toLowerCase().includes(q) || v.id.includes(q))
        vids.push({...v, channel: ch.name});
    }
  }
  if (sort==='dur-asc')  vids.sort((a,b)=>(a.duration_secs??0)-(b.duration_secs??0));
  if (sort==='dur-desc') vids.sort((a,b)=>(b.duration_secs??0)-(a.duration_secs??0));
  if (sort==='size-asc') vids.sort((a,b)=>(a.file_size??0)-(b.file_size??0));
  if (sort==='size-desc')vids.sort((a,b)=>(b.file_size??0)-(a.file_size??0));
  if (sort==='title')    vids.sort((a,b)=>a.title.localeCompare(b.title));
  return vids;
}

function renderGrid() {
  const vids = currentVideos();
  setStatus(`${vids.length} videos`);
  const grid = document.getElementById('grid');
  if (!vids.length) { grid.innerHTML = '<div class="empty">Nothing here.</div>'; return; }
  grid.innerHTML = vids.map(v => {
    const chk = bulkMode ? `<input type="checkbox" ${selected.has(v.id)?'checked':''} onchange="toggleSel('${v.id}',this.checked)">` : '';
    const meta = [
      v.channel !== activeChannel ? esc(v.channel) : null,
      v.duration_secs != null ? fmtDur(v.duration_secs) : null,
      v.file_size != null ? fmtSize(v.file_size) : null,
      v.has_live_chat ? '💬' : null,
      !v.has_video ? '<span style="color:#f87171">no file</span>' : null,
    ].filter(Boolean).join(' · ');
    return `<div class="card${v.watched?' watched':''}">
      <div class="card-title">${chk} ${esc(v.title)}</div>
      <div class="card-meta">${meta||'&nbsp;'}</div>
      <div class="card-foot">
        <button onclick="toggleWatched('${v.id}')">${v.watched?'✓ Watched':'○ Watch'}</button>
      </div>
    </div>`;
  }).join('');
}

async function toggleWatched(id) {
  try { await api(`/api/watched/${id}`, {method:'POST'}); await loadLibrary(); }
  catch(e) { setStatus('Error: ' + e.message); }
}

function toggleBulk() {
  bulkMode = document.getElementById('bulk').checked;
  selected.clear();
  document.getElementById('bulk-actions').style.display = bulkMode ? 'flex' : 'none';
  renderGrid();
}

function toggleSel(id, on) {
  if (on) selected.add(id); else selected.delete(id);
  document.getElementById('sel-count').textContent = selected.size + ' selected';
}

async function bulkWatched(on) {
  for (const id of selected) {
    const db = await api('/api/library');
    const lib = (await db.json()).channels;
    // find current state
  }
  // simpler: just toggle each
  const ids = [...selected];
  await Promise.all(ids.map(id => api(`/api/watched/${id}`, {method:'POST'})));
  selected.clear();
  await loadLibrary();
}

async function startDownload() {
  const url = document.getElementById('dl-url').value.trim();
  if (!url) return;
  try {
    await api('/api/download', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({url})});
    document.getElementById('dl-url').value = '';
    setStatus('Download started…');
  } catch(e) { setStatus('Error: '+e.message); }
}

async function downloadChannel(url) {
  try {
    await api('/api/download', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({url})});
    setStatus('Checking for new videos…');
  } catch(e) { setStatus('Error: '+e.message); }
}

async function rescan() {
  try { await api('/api/rescan', {method:'POST'}); await loadLibrary(); }
  catch(e) { setStatus('Error: '+e.message); }
}

function renderJobs(jobs) {
  const el = document.getElementById('jobs');
  if (!jobs.length) { el.innerHTML=''; return; }
  el.innerHTML = jobs.map(j=>`<div class="job">
    <span class="badge ${j.state}">${j.state}</span>
    <span style="flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(j.label)} — ${esc(j.url)}</span>
    ${j.state==='running'?`<progress value="${j.progress}" max="1"></progress>`:''}
    <span style="font-size:11px;color:var(--muted);max-width:200px;overflow:hidden;text-overflow:ellipsis">${esc(j.last_line)}</span>
  </div>`).join('');
}

// Poll progress every 600ms
let wasRunning = false;
async function pollProgress() {
  try {
    const r = await api('/api/progress');
    const {jobs} = await r.json();
    renderJobs(jobs);
    const running = jobs.some(j=>j.state==='running');
    if (wasRunning && !running) await loadLibrary();
    wasRunning = running;
  } catch {}
}
setInterval(pollProgress, 600);

function fmtDur(s) {
  s=Math.floor(s); const h=Math.floor(s/3600),m=Math.floor((s%3600)/60),sec=s%60;
  return h?`${h}:${p(m)}:${p(sec)}`:`${m}:${p(sec)}`;
}
function fmtSize(b) {
  if(b>=1073741824)return(b/1073741824).toFixed(1)+' GB';
  if(b>=1048576)return Math.round(b/1048576)+' MB';
  return Math.round(b/1024)+' KB';
}
function p(n){return String(n).padStart(2,'0')}
function esc(s){return String(s??'').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;')}

loadLibrary();
</script>
</body>
</html>"#;
