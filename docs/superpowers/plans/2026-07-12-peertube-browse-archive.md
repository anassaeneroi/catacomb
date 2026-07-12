# PeerTube Browse + Archive Implementation Plan (Federation Phase 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user browse a PeerTube peer's channels/videos in both front-ends, play a video when a direct MP4 exists, and archive any video into the local library with one click.

**Architecture:** Browsing stays kind-dispatched. Catacomb peers keep the one-shot `/library` path; PeerTube peers use four new lazy, PeerTube-only web endpoints backed by thin `RemoteClientKind` passthroughs to the existing `PeerTubeClient`. Both UIs gain a two-level nav (channels → paginated videos). Archive reuses `Downloader::start` (lands in `Other`). Media URLs are resolved on-demand (only when Play is clicked).

**Tech Stack:** Rust, axum, eframe/egui, reqwest (blocking), serde/serde_json; the embedded `web_ui/index.html` SPA.

## Global Constraints

- Inherits `docs/superpowers/specs/2026-07-12-peertube-browse-archive-design.md`.
- The new endpoints are **PeerTube-only**: on a catacomb remote they return `400 Bad Request`. UIs dispatch on `kind` and never call them for a catacomb peer.
- Archive reuses the exact `Downloader::start(url, &classify_url(&url), false, DownloadQuality::Best, false, None)` shape `post_download` uses; `start` returns `()`, so the web response is `202 "ok"`. Archived PeerTube URLs classify as `Platform::Other` (no new platform, no override).
- On-demand media resolution — never resolve a video's MP4 until Play is clicked (eager would be one extra HTTP call per listed video).
- The web SPA is one embedded file; **`cargo build` does not catch JS syntax errors** — after editing `web_ui/index.html`, run `awk '/<script>/{f=1;next}/<\/script>/{f=0}f' src/web_ui/index.html > /tmp/spa.js && node --check /tmp/spa.js`.
- New UI must use existing theme CSS variables (web) / existing egui idioms (desktop); no CDN assets (offline-first).
- Blocking PeerTube calls run on `tokio::task::spawn_blocking` in web handlers (like `get_remote_library`) and on `std::thread::spawn` + `mpsc` + `request_repaint` in desktop (like `start_remote_fetch`).
- Commits SSH-signed: `export SSH_AUTH_SOCK=/tmp/luna-ssh-agent.sock`. End messages with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

---

### Task 1: `RemoteClientKind` PeerTube passthroughs + `RemoteChannelInfo` serialize

Atomic: add the four passthroughs the web handlers and desktop threads call, make `RemoteChannelInfo` JSON-serializable, unit-test the kind-guard. Compiles; nothing calls the passthroughs yet.

**Files:**
- Modify: `src/remote.rs` — `impl RemoteClientKind` (after `kind()`, ~line 190) + test mod.
- Modify: `src/peertube.rs` — add `serde::Serialize` to `RemoteChannelInfo` (~line 27).

**Interfaces:**
- Consumes: `crate::peertube::PeerTubeClient::{list_channels, channel_videos, video_media, watch_url}`, `crate::peertube::RemoteChannelInfo`, `crate::remote::RemoteVideo`.
- Produces on `RemoteClientKind`:
  - `pt_channels(&self) -> Result<Vec<crate::peertube::RemoteChannelInfo>, String>`
  - `pt_channel_videos(&self, handle: &str, page: usize) -> Result<Vec<RemoteVideo>, String>`
  - `pt_video_media(&self, uuid: &str) -> Result<Option<String>, String>`
  - `pt_watch_url(&self, uuid: &str) -> Result<String, String>`
  - Each returns `Err("not a PeerTube remote".into())` for the `Catacomb` arm.

- [ ] **Step 1: Make `RemoteChannelInfo` serializable**

In `src/peertube.rs`, change the derive on `RemoteChannelInfo` (~line 27) from:
```rust
#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)] // consumed by the browse UI in phase 3
pub struct RemoteChannelInfo {
```
to:
```rust
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct RemoteChannelInfo {
```
(Drop the `#[allow(dead_code)]` — phase 3 consumes it. Its fields `handle, display_name, video_count, avatar_url` are already the snake_case names the UIs expect.)

- [ ] **Step 2: Write the failing passthrough unit test**

Append to `src/remote.rs`'s `#[cfg(test)] mod tests`:
```rust
#[test]
fn peertube_passthroughs_reject_catacomb() {
    use crate::config::{RemoteKind, RemoteSection};
    let cat = RemoteSection {
        name: "c".into(), url: "http://p:8081".into(),
        kind: RemoteKind::Catacomb, username: None, password: None,
    };
    let k = RemoteClientKind::from_section(&cat);
    assert!(k.pt_channels().is_err());
    assert!(k.pt_channel_videos("h", 0).is_err());
    assert!(k.pt_video_media("u").is_err());
    assert!(k.pt_watch_url("u").is_err());
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test --release peertube_passthroughs 2>&1 | grep -E "cannot find|no method|test result"`
Expected: FAIL — `pt_channels`/etc. not found.

- [ ] **Step 4: Add the passthroughs**

In `src/remote.rs`, inside `impl RemoteClientKind` (after the `kind()` method, before the closing `}`):
```rust
    pub fn pt_channels(&self) -> Result<Vec<crate::peertube::RemoteChannelInfo>, String> {
        match self {
            Self::Peertube(p) => p.list_channels(),
            Self::Catacomb(_) => Err("not a PeerTube remote".into()),
        }
    }
    pub fn pt_channel_videos(&self, handle: &str, page: usize) -> Result<Vec<RemoteVideo>, String> {
        match self {
            Self::Peertube(p) => p.channel_videos(handle, page),
            Self::Catacomb(_) => Err("not a PeerTube remote".into()),
        }
    }
    pub fn pt_video_media(&self, uuid: &str) -> Result<Option<String>, String> {
        match self {
            Self::Peertube(p) => p.video_media(uuid),
            Self::Catacomb(_) => Err("not a PeerTube remote".into()),
        }
    }
    pub fn pt_watch_url(&self, uuid: &str) -> Result<String, String> {
        match self {
            Self::Peertube(p) => Ok(p.watch_url(uuid)),
            Self::Catacomb(_) => Err("not a PeerTube remote".into()),
        }
    }
```
(`RemoteVideo` is already in scope — it's defined in this module.)

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test --release peertube_passthroughs 2>&1 | grep "test result"`
Expected: `test result: ok. 1 passed`.

- [ ] **Step 6: Build + full tests**

Run: `cargo build --release 2>&1 | grep -E "^error" | head` → no output.
Run: `cargo test --release 2>&1 | grep "test result"` → all `ok` (+1 unit vs phase 2).

- [ ] **Step 7: Commit**

```bash
export SSH_AUTH_SOCK=/tmp/luna-ssh-agent.sock
git add src/remote.rs src/peertube.rs
git commit -m "feat(remote): PeerTube browse passthroughs on RemoteClientKind

pt_channels/pt_channel_videos/pt_video_media/pt_watch_url dispatch to the
PeerTubeClient; Err for a catacomb remote. RemoteChannelInfo now Serialize.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Web browse + archive endpoints

**Files:**
- Modify: `src/web.rs` — four handlers + `PageQuery`/`ArchiveBody` + routes (near line 3503).
- Modify: `tests/api.rs` — integration coverage (model on `remotes_editor_put_get_roundtrip`, line 529).

**Interfaces:**
- Consumes: Task 1 passthroughs; `classify_url` (already imported, line 50), `DownloadQuality` (imported, line 48), `Query`/`Path`/`State`/`Json` (imported, line 32).
- Produces routes: `GET /api/remotes/:id/channels`, `GET /api/remotes/:id/channels/:handle/videos`, `GET /api/remotes/:id/videos/:uuid/media`, `POST /api/remotes/:id/archive`.

- [ ] **Step 1: Add the handlers**

In `src/web.rs`, after `get_remote_library` (ends ~line 2413), add:
```rust
#[derive(Deserialize)]
struct PageQuery {
    #[serde(default)]
    page: usize,
}

#[derive(Deserialize)]
struct ArchiveBody {
    uuid: String,
}

/// Guard: fetch the remote and require it be a PeerTube kind. Returns the
/// cloned Arc on success, or the ready-made error Response.
fn require_peertube(
    state: &Arc<WebState>,
    id: usize,
) -> Result<std::sync::Arc<crate::remote::RemoteClientKind>, Response> {
    let remote = state.remotes.read().unwrap().get(id).cloned();
    let Some(remote) = remote else {
        return Err((StatusCode::NOT_FOUND, "no such remote").into_response());
    };
    if remote.kind() != crate::config::RemoteKind::Peertube {
        return Err((StatusCode::BAD_REQUEST, "not a PeerTube remote").into_response());
    }
    Ok(remote)
}

/// `GET /api/remotes/:id/channels` — a PeerTube peer's channel list.
async fn get_remote_channels(
    State(state): State<Arc<WebState>>,
    Path(id): Path<usize>,
) -> Response {
    let remote = match require_peertube(&state, id) { Ok(r) => r, Err(e) => return e };
    match tokio::task::spawn_blocking(move || remote.pt_channels()).await {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_GATEWAY, format!("remote error: {e}")).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "remote task failed").into_response(),
    }
}

/// `GET /api/remotes/:id/channels/:handle/videos?page=N` — one page (24) of a
/// channel's videos. `:handle` may be `name@host` (axum percent-decodes it).
async fn get_remote_channel_videos(
    State(state): State<Arc<WebState>>,
    Path((id, handle)): Path<(usize, String)>,
    Query(q): Query<PageQuery>,
) -> Response {
    let remote = match require_peertube(&state, id) { Ok(r) => r, Err(e) => return e };
    let page = q.page;
    match tokio::task::spawn_blocking(move || remote.pt_channel_videos(&handle, page)).await {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_GATEWAY, format!("remote error: {e}")).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "remote task failed").into_response(),
    }
}

/// `GET /api/remotes/:id/videos/:uuid/media` — resolve a directly-playable MP4.
/// `204 No Content` when the video is HLS-only.
async fn get_remote_video_media(
    State(state): State<Arc<WebState>>,
    Path((id, uuid)): Path<(usize, String)>,
) -> Response {
    let remote = match require_peertube(&state, id) { Ok(r) => r, Err(e) => return e };
    match tokio::task::spawn_blocking(move || remote.pt_video_media(&uuid)).await {
        Ok(Ok(Some(url))) => Json(serde_json::json!({ "url": url })).into_response(),
        Ok(Ok(None)) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(e)) => (StatusCode::BAD_GATEWAY, format!("remote error: {e}")).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "remote task failed").into_response(),
    }
}

/// `POST /api/remotes/:id/archive` `{uuid}` — queue a PeerTube video into the
/// local library via the shared downloader (lands in `Other`).
async fn post_remote_archive(
    State(state): State<Arc<WebState>>,
    Path(id): Path<usize>,
    Json(body): Json<ArchiveBody>,
) -> Response {
    let remote = match require_peertube(&state, id) { Ok(r) => r, Err(e) => return e };
    let watch_url = match remote.pt_watch_url(&body.uuid) {
        Ok(u) => u,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let info = classify_url(&watch_url);
    {
        let mut dl = state.downloader.lock_recover();
        dl.start(watch_url, &info, false, DownloadQuality::Best, false, None);
    }
    (StatusCode::ACCEPTED, "ok").into_response()
}
```

- [ ] **Step 2: Register the routes**

In `src/web.rs`, after the existing remotes routes (line ~3503):
```rust
        .route("/api/remotes/:id/channels", get(get_remote_channels))
        .route("/api/remotes/:id/channels/:handle/videos", get(get_remote_channel_videos))
        .route("/api/remotes/:id/videos/:uuid/media", get(get_remote_video_media))
        .route("/api/remotes/:id/archive", post(post_remote_archive))
```

- [ ] **Step 3: Write the failing integration test**

Append to `tests/api.rs`:
```rust
#[test]
fn peertube_browse_endpoints_kind_guarded() {
    if !have_curl() { eprintln!("skip: no curl"); return; }
    let s = Server::start();

    // A peertube remote pointing at a dead port, and a catacomb remote.
    let body = r#"[
        {"name":"pt","url":"http://127.0.0.1:59999","kind":"peertube"},
        {"name":"cat","url":"http://127.0.0.1:59998","kind":"catacomb"}
    ]"#;
    assert_eq!(s.put("/api/remotes", body).0, 200);

    // PeerTube channels endpoint on the peertube remote: route exists, but the
    // host is unreachable → 502 (NOT 404-route-missing, NOT 400-wrong-kind).
    let (code, _) = s.get("/api/remotes/0/channels");
    assert_eq!(code, 502, "unreachable peertube host → bad gateway");

    // Same endpoint on the catacomb remote → 400 (kind guard).
    let (code, _) = s.get("/api/remotes/1/channels");
    assert_eq!(code, 400, "channels on a catacomb remote is rejected");

    // Archive on the catacomb remote is also kind-guarded.
    let (code, _) = s.post("/api/remotes/1/archive", r#"{"uuid":"abc"}"#);
    assert_eq!(code, 400, "archive on a catacomb remote is rejected");

    // Unknown remote id → 404.
    let (code, _) = s.get("/api/remotes/9/channels");
    assert_eq!(code, 404);
}
```

- [ ] **Step 4: Run to verify it fails**

Run: `cargo test --release peertube_browse_endpoints 2>&1 | grep -E "test result|assert|404|400|502"`
Expected: FAIL before Step 1/2 are in (routes missing → 404 where 502/400 expected). If Steps 1–2 already applied, it should pass — that's fine; proceed.

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test --release peertube_browse_endpoints 2>&1 | grep "test result"`
Expected: `test result: ok. 1 passed`. (Needs `curl`; skips otherwise.)

- [ ] **Step 6: Build + full tests**

Run: `cargo build --release 2>&1 | grep -E "^error" | head` → no output.
Run: `cargo test --release 2>&1 | grep "test result"` → all `ok`.

- [ ] **Step 7: Commit**

```bash
export SSH_AUTH_SOCK=/tmp/luna-ssh-agent.sock
git add src/web.rs tests/api.rs
git commit -m "feat(web): PeerTube browse + archive endpoints (kind-guarded)

GET channels / channel videos (paged) / video media (204 = HLS-only);
POST archive routes watch_url through the shared downloader (lands in Other).

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Web UI — PeerTube browse view + archive

**Files:**
- Modify: `src/web_ui/index.html` — sidebar dispatch + a PeerTube browse view + JS.

**Interfaces:**
- Consumes: Task 2 endpoints; existing helpers `api()`, `esc()`, `setStatus()`, `playVideo()`, `fmtDur`/`format_duration` equivalent, `remotes` array (each `{id,name,url,kind}`), `renderSidebar()`.

The current sidebar renders each peer as `onclick="enterRemote(${r.id})"` (line ~797). Catacomb peers keep `enterRemote`; PeerTube peers get `openPeertube`.

- [ ] **Step 1: Dispatch the sidebar click on kind**

In `src/web_ui/index.html`, in `renderSidebar` (~line 797), change:
```javascript
    remotes.forEach(r=>{h+=`<div class="ch-item${remoteMode===r.id?' active':''}" onclick="enterRemote(${r.id})" title="${esc(r.url)}">🌐 ${esc(r.name)}</div>`;});
```
to:
```javascript
    remotes.forEach(r=>{const fn=r.kind==='peertube'?'openPeertube':'enterRemote';const act=(remoteMode===r.id||ptState.id===r.id)?' active':'';h+=`<div class="ch-item${act}" onclick="${fn}(${r.id})" title="${esc(r.url)}">🌐 ${esc(r.name)}</div>`;});
```

- [ ] **Step 2: Add a container for the PeerTube view**

Find the main content container that holds the video grid (the element `renderGrid()` writes into — search for `getElementById('grid')` or the grid container id). Immediately after that grid element in the HTML, add a sibling overlay container:
```html
<div id="ptView" style="display:none;padding:16px;overflow:auto"></div>
```
(If the grid container id differs, place `ptView` as a sibling of it inside the same scroll region so hiding the grid and showing `ptView` swaps the main area.)

- [ ] **Step 3: Add the PeerTube browse JS**

In the `<script>` block (near the other remote functions, ~line 756), add:
```javascript
/* ── PeerTube browse (lazy two-level nav) ───────────────────────── */
let ptState = { id:null, channel:null, page:0, videos:[], done:false };
function ptShow(on){
  const g=document.getElementById('grid'); // the local/remote video grid
  const v=document.getElementById('ptView');
  if(g)g.style.display=on?'none':'';
  if(v)v.style.display=on?'':'none';
}
async function openPeertube(id){
  const r=remotes.find(x=>x.id===id); if(!r)return;
  ptState={ id, channel:null, page:0, videos:[], done:false };
  remoteMode=null; document.body.classList.remove('remote-mode');
  closeSidebar(); renderSidebar(); ptShow(true);
  setStatus('Connecting to '+r.name+'…');
  document.getElementById('ptView').innerHTML='<div class="muted">Loading channels…</div>';
  try{
    const chs=await(await api('/api/remotes/'+id+'/channels')).json();
    renderPtChannels(r,chs);
    setStatus('Viewing '+r.name+' (read-only)');
  }catch(e){document.getElementById('ptView').innerHTML='<div class="muted">Error: '+esc(e.message)+'</div>';}
}
function exitPeertube(){
  ptState={ id:null, channel:null, page:0, videos:[], done:false };
  ptShow(false); libraryEtag=null; renderSidebar(); loadLibrary();
  setStatus('Back to your library');
}
function renderPtChannels(r,chs){
  let h=`<div class="ch-item" onclick="exitPeertube()">← Back to my library</div>`;
  h+=`<h3 style="margin:8px 0">🌐 ${esc(r.name)} — channels</h3>`;
  if(!chs.length)h+='<div class="muted">No channels.</div>';
  chs.forEach(c=>{
    const n=c.video_count!=null?` (${c.video_count})`:'';
    h+=`<div class="ch-item" onclick='openPtChannel(${JSON.stringify(c.handle)})'>${esc(c.display_name||c.handle)}${esc(n)}</div>`;
  });
  document.getElementById('ptView').innerHTML=h;
}
async function openPtChannel(handle){
  ptState.channel=handle; ptState.page=0; ptState.videos=[]; ptState.done=false;
  await loadPtPage(true);
}
async function loadPtPage(first){
  const id=ptState.id, handle=ptState.channel;
  try{
    const vids=await(await api('/api/remotes/'+id+'/channels/'+encodeURIComponent(handle)+'/videos?page='+ptState.page)).json();
    ptState.videos.push(...vids);
    if(vids.length<24)ptState.done=true; else ptState.page++;
    renderPtVideos();
  }catch(e){setStatus('Error: '+e.message);}
}
function renderPtVideos(){
  let h=`<div class="ch-item" onclick='openPeertube(${ptState.id})'>← Back to channels</div>`;
  h+=`<h3 style="margin:8px 0">${esc(ptState.channel)}</h3>`;
  h+='<div class="pt-grid" style="display:grid;grid-template-columns:repeat(auto-fill,minmax(220px,1fr));gap:12px">';
  ptState.videos.forEach((v,i)=>{
    const dur=v.duration_secs?fmtDur(v.duration_secs):'';
    const thumb=v.thumb_url?`<img src="${esc(v.thumb_url)}" style="width:100%;border-radius:6px" loading="lazy">`:'';
    h+=`<div class="pt-card" data-i="${i}" style="background:var(--card);border:1px solid var(--border);border-radius:8px;padding:8px">
      ${thumb}
      <div style="font-weight:600;margin:6px 0;font-size:13px">${esc(v.title)}</div>
      <div class="muted" style="font-size:11px">${esc(dur)}</div>
      <div style="display:flex;gap:6px;margin-top:6px">
        <button class="pt-play" data-i="${i}" data-uuid="${esc(v.id)}">▶ Play</button>
        <button class="pt-arch" data-uuid="${esc(v.id)}">⬇ Archive</button>
      </div>
      <span class="pt-note muted" data-i="${i}" style="font-size:11px"></span>
    </div>`;
  });
  h+='</div>';
  if(!ptState.done)h+='<div style="margin:12px 0"><button onclick="loadPtPage(false)">Load more</button></div>';
  const box=document.getElementById('ptView'); box.innerHTML=h;
  box.querySelectorAll('.pt-play').forEach(b=>b.onclick=()=>ptPlay(b.dataset.uuid,+b.dataset.i));
  box.querySelectorAll('.pt-arch').forEach(b=>b.onclick=()=>ptArchive(b.dataset.uuid,b));
}
async function ptPlay(uuid,i){
  const note=document.querySelector('.pt-note[data-i="'+i+'"]');
  const btn=document.querySelector('.pt-play[data-i="'+i+'"]');
  if(note)note.textContent='resolving…';
  try{
    const r=await api('/api/remotes/'+ptState.id+'/videos/'+encodeURIComponent(uuid)+'/media');
    if(r.status===204){ if(note)note.textContent='HLS-only — archive to watch'; if(btn)btn.disabled=true; return; }
    const j=await r.json();
    if(note)note.textContent='';
    playVideo({title:ptState.videos[i].title, direct_url:j.url});
  }catch(e){ if(note)note.textContent='✗ '+e.message; }
}
async function ptArchive(uuid,btn){
  btn.disabled=true; btn.textContent='⬇ Queued';
  try{
    await api('/api/remotes/'+ptState.id+'/archive',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({uuid})});
    setStatus('Archiving — see Downloads');
  }catch(e){ btn.disabled=false; btn.textContent='⬇ Archive'; setStatus('Archive failed: '+e.message); }
}
```

**Adapt to the file's real helpers** (verify each before finalizing):
- `document.getElementById('grid')` — replace `'grid'` with the actual video-grid container id used by `renderGrid()`.
- `fmtDur(secs)` — use the file's existing duration formatter (grep for how the local grid formats `duration`); if none, inline `Math floor` mm:ss.
- `playVideo({...})` — match the real player entry point's signature (grep `function playVideo`). The player needs a direct media URL; pass whatever field name it reads (e.g. `direct_url`/`video_url`). If the player takes a video object from the library, build the minimal object it needs.

- [ ] **Step 4: Syntax-check the SPA JS**

Run: `awk '/<script>/{f=1;next}/<\/script>/{f=0}f' src/web_ui/index.html > /tmp/spa.js && node --check /tmp/spa.js && echo "JS OK"`
Expected: `JS OK`.

- [ ] **Step 5: Build + live verify**

Run: `cargo build --release 2>&1 | grep -E "^error" | head` → no output.
Then verify end-to-end against a scratch dir (see CLAUDE.md "Running against a real library"): start `--web <port>`, `PUT /api/remotes` a peertube peer (`https://framatube.org`), open it in the sidebar, confirm the channel list loads, a channel opens with a video grid + Load more, Play resolves and opens the player (or shows the HLS-only note), and Archive returns and the job appears in Downloads. If no network, at minimum confirm the sidebar dispatch shows the browse view and errors render inline (no crash).

- [ ] **Step 6: Commit**

```bash
export SSH_AUTH_SOCK=/tmp/luna-ssh-agent.sock
git add src/web_ui/index.html
git commit -m "feat(web-ui): PeerTube browse view + per-video archive

Two-level lazy nav (channels → paged videos), on-demand Play (HLS-only
disabled), one-click Archive to the local library.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: Desktop UI — PeerTube browse + archive

**Files:**
- Modify: `src/app.rs` — `App` state, `remotes_screen` PeerTube branch, three fetch threads, `update()` drains.

**Interfaces:**
- Consumes: Task 1 passthroughs on `Arc<RemoteClientKind>`; `crate::platform::classify_url`; `crate::downloader::DownloadQuality`; existing `self.downloader`, `self.remotes`, `self.remote_status`, `self.play_remote_url`, `format_duration`.

- [ ] **Step 1: Add PeerTube browse state to `App`**

In the `App` struct (near `remote_test_rx`, ~line 274), add:
```rust
    /// PeerTube browse (desktop two-level nav). `pt_remote` is the selected
    /// peer index; None means not browsing a PeerTube peer.
    pt_remote: Option<usize>,
    pt_channels: Option<Vec<crate::peertube::RemoteChannelInfo>>,
    pt_channel: Option<String>,
    pt_videos: Vec<crate::remote::RemoteVideo>,
    pt_page: usize,
    pt_done: bool,
    pt_channels_rx: Option<Receiver<Result<Vec<crate::peertube::RemoteChannelInfo>, String>>>,
    pt_videos_rx: Option<Receiver<Result<Vec<crate::remote::RemoteVideo>, String>>>,
    pt_media_rx: Option<Receiver<Result<Option<String>, String>>>,
```
In the constructor (near `remote_test_rx: None`, ~line 703), add:
```rust
            pt_remote: None,
            pt_channels: None,
            pt_channel: None,
            pt_videos: Vec::new(),
            pt_page: 0,
            pt_done: false,
            pt_channels_rx: None,
            pt_videos_rx: None,
            pt_media_rx: None,
```

- [ ] **Step 2: Add the fetch/archive helpers**

In `impl App`, after `start_remote_test` (~line 2585), add:
```rust
    /// Begin browsing PeerTube peer `idx`: fetch its channel list on a thread.
    fn start_pt_browse(&mut self, idx: usize) {
        let Some(client) = self.remotes.get(idx).cloned() else { return };
        self.pt_remote = Some(idx);
        self.pt_channels = None;
        self.pt_channel = None;
        self.pt_videos.clear();
        self.pt_page = 0;
        self.pt_done = false;
        self.remote_status = format!("Connecting to {}…", client.name());
        let (tx, rx) = std::sync::mpsc::channel();
        self.pt_channels_rx = Some(rx);
        let ctx = self.egui_ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(client.pt_channels());
            ctx.request_repaint();
        });
    }

    /// Fetch one page of the selected channel's videos on a thread. `reset`
    /// clears the accumulated list (new channel); otherwise it appends.
    fn start_pt_videos(&mut self, reset: bool) {
        let (Some(idx), Some(handle)) = (self.pt_remote, self.pt_channel.clone()) else { return };
        let Some(client) = self.remotes.get(idx).cloned() else { return };
        if reset { self.pt_videos.clear(); self.pt_page = 0; self.pt_done = false; }
        let page = self.pt_page;
        self.remote_status = "Loading videos…".to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        self.pt_videos_rx = Some(rx);
        let ctx = self.egui_ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(client.pt_channel_videos(&handle, page));
            ctx.request_repaint();
        });
    }

    /// Resolve a video's playable MP4 on a thread; result drained in update().
    fn start_pt_play(&mut self, uuid: String) {
        let Some(idx) = self.pt_remote else { return };
        let Some(client) = self.remotes.get(idx).cloned() else { return };
        self.remote_status = "Resolving…".to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        self.pt_media_rx = Some(rx);
        let ctx = self.egui_ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(client.pt_video_media(&uuid));
            ctx.request_repaint();
        });
    }

    /// Queue a PeerTube video into the local library via the shared downloader.
    fn start_pt_archive(&mut self, uuid: &str) {
        let Some(idx) = self.pt_remote else { return };
        let Some(client) = self.remotes.get(idx).cloned() else { return };
        match client.pt_watch_url(uuid) {
            Ok(url) => {
                let info = crate::platform::classify_url(&url);
                self.downloader.start(url, &info, false, crate::downloader::DownloadQuality::Best, false, None);
                self.remote_status = "Archiving — see Downloads".to_string();
            }
            Err(e) => self.remote_status = e,
        }
    }
```

- [ ] **Step 3: Render the PeerTube branch in `remotes_screen`**

In `remotes_screen` (~line 2588), the peer-selection row already lists all remotes. Replace the `start_remote_fetch(i)` dispatch at the bottom (`if let Some(i) = select_remote { self.start_remote_fetch(i); }`) with a kind check:
```rust
        if let Some(i) = select_remote {
            match self.remotes.get(i).map(|r| r.kind()) {
                Some(crate::config::RemoteKind::Peertube) => self.start_pt_browse(i),
                _ => self.start_remote_fetch(i),
            }
        }
```
Then, inside the `CentralPanel` closure, render the PeerTube nav when `self.pt_remote == Some(i)` for the selected peer. Add this block right after the existing `ScrollArea::vertical().show(...)` that renders `self.remote_library` — guard the whole PeerTube view on `self.pt_remote.is_some()` and the catacomb library view on `self.pt_remote.is_none()`. Collect actions into locals and apply after the closure:
```rust
        // (declare near the top of remotes_screen, beside select_remote/play_url)
        let mut open_channel: Option<String> = None;
        let mut load_more = false;
        let mut pt_play: Option<String> = None;
        let mut pt_arch: Option<String> = None;
        let mut back_to_channels = false;
```
PeerTube view (inside the CentralPanel, after the catacomb `ScrollArea`):
```rust
            if self.pt_remote.is_some() {
                if self.pt_channels_rx.is_some() || self.pt_videos_rx.is_some() || self.pt_media_rx.is_some() {
                    ctx.request_repaint();
                }
                egui::ScrollArea::vertical().id_source("pt-scroll").show(ui, |ui| {
                    if self.pt_channel.is_none() {
                        // Channel list.
                        match &self.pt_channels {
                            None => { ui.label(egui::RichText::new("Loading channels…").weak()); }
                            Some(chs) if chs.is_empty() => { ui.label(egui::RichText::new("No channels.").weak()); }
                            Some(chs) => {
                                for c in chs {
                                    let label = if let Some(n) = c.video_count {
                                        format!("{} ({})", c.display_name, n)
                                    } else { c.display_name.clone() };
                                    if ui.selectable_label(false, label).clicked() {
                                        open_channel = Some(c.handle.clone());
                                    }
                                }
                            }
                        }
                    } else {
                        // Video list for the selected channel.
                        if ui.button("← Back to channels").clicked() { back_to_channels = true; }
                        ui.heading(self.pt_channel.clone().unwrap_or_default());
                        for v in &self.pt_videos {
                            ui.horizontal(|ui| {
                                if ui.button("▶ Play").clicked() { pt_play = Some(v.id.clone()); }
                                if ui.button("⬇ Archive").clicked() { pt_arch = Some(v.id.clone()); }
                                let dur = v.duration_secs.map(format_duration).unwrap_or_default();
                                ui.label(format!("{}  {}", v.title, dur));
                            });
                        }
                        if !self.pt_done && !self.pt_videos.is_empty() {
                            if ui.button("Load more").clicked() { load_more = true; }
                        }
                    }
                });
            }
```
After the `CentralPanel` closure (beside the existing `if let Some(u) = play_url {...}`), apply:
```rust
        if let Some(h) = open_channel { self.pt_channel = Some(h); self.start_pt_videos(true); }
        if back_to_channels { self.pt_channel = None; self.pt_videos.clear(); }
        if load_more { self.pt_page += 1; self.start_pt_videos(false); }
        if let Some(u) = pt_play { self.start_pt_play(u); }
        if let Some(u) = pt_arch { self.start_pt_archive(&u); }
```
Also gate the catacomb library `ScrollArea` on `self.pt_remote.is_none()` so a PeerTube peer doesn't show a stale catacomb library. And update the screen's intro label to mention PeerTube (optional).

- [ ] **Step 4: Drain the PeerTube results in `update()`**

In `update()`, after the `remote_test_rx` drain (~line 5030), add:
```rust
        if let Some(res) = self.pt_channels_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.pt_channels_rx = None;
            match res {
                Ok(chs) => { self.remote_status = format!("{} channels", chs.len()); self.pt_channels = Some(chs); }
                Err(e) => { self.remote_status = format!("Error: {e}"); self.pt_channels = Some(Vec::new()); }
            }
            ctx.request_repaint();
        }
        if let Some(res) = self.pt_videos_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.pt_videos_rx = None;
            match res {
                Ok(vids) => {
                    if vids.len() < 24 { self.pt_done = true; } else { self.pt_page += 1; }
                    self.remote_status = format!("{} videos", self.pt_videos.len() + vids.len());
                    self.pt_videos.extend(vids);
                }
                Err(e) => self.remote_status = format!("Error: {e}"),
            }
            ctx.request_repaint();
        }
        if let Some(res) = self.pt_media_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.pt_media_rx = None;
            match res {
                Ok(Some(url)) => { self.remote_status.clear(); self.play_remote_url(&url); }
                Ok(None) => self.remote_status = "HLS-only — archive to watch".to_string(),
                Err(e) => self.remote_status = format!("Error: {e}"),
            }
            ctx.request_repaint();
        }
```
**Note on `pt_page`:** `start_pt_videos` reads `self.pt_page` *before* spawning, and the drain advances it on success; `load_more` also increments before calling `start_pt_videos(false)`. To avoid double-advance, `load_more` must NOT pre-increment — remove the `self.pt_page += 1;` from the `load_more` apply-block (Step 3) and let the drain own advancement. Correct Step 3's apply line to: `if load_more { self.start_pt_videos(false); }` — the drain's `self.pt_page += 1` on the previous page already moved it forward. (Verify: page 0 fetched → drain sets page=1 → Load more fetches page 1 → drain sets page=2. Correct.)

- [ ] **Step 5: Build + full tests**

Run: `cargo build --release 2>&1 | grep -E "^error" | head` → no output.
Run: `cargo test --release 2>&1 | grep "test result"` → all `ok`.

- [ ] **Step 6: Manual verify (desktop)**

Launch the GUI (XWayland recipe in HANDOFF.md if screenshotting), open Remote libraries, select a PeerTube peer (`https://framatube.org`), confirm the channel list loads, open a channel → video rows + Load more, Play a direct-MP4 video (mpv launches) and confirm an HLS-only one shows the note, Archive one and confirm "Archiving — see Downloads" + the job in the download queue. Selecting a catacomb peer still browses its library normally.

- [ ] **Step 7: Commit**

```bash
export SSH_AUTH_SOCK=/tmp/luna-ssh-agent.sock
git add src/app.rs
git commit -m "feat(desktop): PeerTube browse + per-video archive

Two-level lazy nav in the Remotes screen (channels → paged videos),
on-demand Play via mpv (HLS-only noted), one-click Archive.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- Kind-dispatched browse + PeerTube passthroughs → Task 1. ✓
- Four backend endpoints (channels, channel videos paged, media 204=HLS, archive 202) + kind guard → Task 2. ✓
- Web two-level nav + on-demand Play + Archive → Task 3. ✓
- Desktop two-level nav + mpv Play + Archive → Task 4. ✓
- Archive reuses `Downloader::start` → `Other` → Tasks 2/4 (identical `classify_url` + `start` shape). ✓
- HLS-only handling (204 → disable/note) → Task 3 Step 3 (`ptPlay`), Task 4 Step 4 (`pt_media_rx` drain). ✓
- On-demand media resolution → both Play paths resolve only on click. ✓
- Testing: passthrough kind-guard unit (Task 1), endpoint route/guard integration (Task 2), manual browse (Tasks 3/4). ✓

**Placeholder scan:** The three "adapt to the file's real helpers" bullets in Task 3 Step 3 name exact integration points (grid container id, duration formatter, `playVideo` signature) that must be read from the file — they are verify-then-use instructions with fallbacks, not vague work. All logic steps carry concrete code.

**Type consistency:** `pt_channels`/`pt_channel_videos`/`pt_video_media`/`pt_watch_url` are named and typed identically where defined (Task 1) and used (Tasks 2/4). `RemoteChannelInfo` fields (`handle`, `display_name`, `video_count`, `avatar_url`) and `RemoteVideo` fields (`id`, `title`, `channel`, `thumb_url`, `duration_secs`) match the JSON the web JS reads and the egui rendering. `PageQuery.page`/`ArchiveBody.uuid` match the routes and the JS request bodies. The `pt_page` double-advance hazard is called out and resolved in Task 4 Step 4.
