# PeerTube Browse + Archive — Design (Federation Phase 3)

> Third and final phase of the federation/PeerTube project. Phase 1 shipped the
> backend `PeerTubeClient` (`docs/superpowers/specs/2026-07-10-peertube-client-backend-design.md`);
> phase 2 shipped the kind-aware remote editor
> (`docs/superpowers/specs/2026-07-10-federation-editor-phase2-kind-aware-design.md`).
> This phase adds the browse UI + per-video archive action to both front-ends.

## Goal

Let a user browse a configured PeerTube peer's channels and videos from inside
Catacomb (both the web SPA and the desktop GUI), play a video inline when the
instance exposes a direct MP4, and archive any video into the local library with
one click. Read-only browsing, one-video-at-a-time archiving.

## Locked decisions (from brainstorming)

1. **Lazy two-level navigation** — list channels, then click a channel to load
   its videos a page at a time. Not a one-shot whole-library load.
2. **Per-video archive only** — each video row has an Archive button. No
   bulk/whole-channel archive in this phase.
3. **Archive destination = existing `Other` platform** — PeerTube is federated
   with no fixed domain, so `Platform::from_url` classifies a `watch_url` as
   `Other`, landing archived videos in `other/<uploader>/`. No new `Platform`
   variant, no download-path override.
4. **Both front-ends in one spec** — web + desktop at parity, shared backend
   endpoints.
5. **On-demand media resolution** — resolve a video's playable MP4 only when
   Play is clicked, not eagerly for every listed video (eager would fire one
   extra HTTP call per video per page).

## Background: what already exists

`PeerTubeClient` (phase 1, `src/peertube.rs`) is a blocking client with:

- `list_channels() -> Result<Vec<RemoteChannelInfo>, String>` —
  `RemoteChannelInfo { handle, display_name, video_count: Option<u64>, avatar_url: Option<String> }`.
- `channel_videos(handle, page) -> Result<Vec<RemoteVideo>, String>` — page size
  24, newest first. Returns `crate::remote::RemoteVideo { id (uuid), title,
  channel, video_url: None, thumb_url, duration_secs }`. `video_url` is
  deliberately `None` here — the playable URL is resolved separately.
- `video_media(uuid) -> Result<Option<String>, String>` — the direct MP4 URL, or
  `None` when the video is HLS-only.
- `watch_url(uuid) -> String` — `{api_base}/w/{uuid}`, the canonical page URL
  handed to yt-dlp.

`RemoteClientKind` (phase 2, `src/remote.rs`) wraps `Catacomb(RemoteClient)` /
`Peertube(PeerTubeClient)`. Both front-ends hold `Vec<Arc<RemoteClientKind>>`
(web behind a `RwLock`). Browsing dispatches on kind; the phase-2 stopgap for a
Peertube remote is a "browsing arrives in a later update" message, replaced here.

The existing **catacomb** browse path is untouched: `GET /api/remotes/:id/library`
returns the whole peer library and the SPA swaps its `library` array to reuse the
normal grid (`enterRemote`/`exitRemote` in `index.html`); desktop uses
`start_remote_fetch` → `remote_library`.

## Architecture

Browsing remains **kind-dispatched**. Catacomb peers keep the one-shot `/library`
path. Peertube peers use new lazy endpoints and a new two-level browse view in
each UI. The new endpoints are Peertube-only: called against a catacomb remote
they return `400 Bad Request` ("not a PeerTube remote").

`RemoteClientKind` gains thin passthroughs used by the new web handlers and the
desktop threads:

```rust
impl RemoteClientKind {
    // Returns Err for the Catacomb arm ("not a PeerTube remote").
    pub fn pt_channels(&self) -> Result<Vec<crate::peertube::RemoteChannelInfo>, String>;
    pub fn pt_channel_videos(&self, handle: &str, page: usize) -> Result<Vec<RemoteVideo>, String>;
    pub fn pt_video_media(&self, uuid: &str) -> Result<Option<String>, String>;
    pub fn pt_watch_url(&self, uuid: &str) -> Result<String, String>;
}
```

(Alternatively the handlers `match` on the arm directly; the passthroughs keep the
`unreachable!()` noise out of both front-ends and give one kind-guard site.)

## Backend endpoints (`src/web.rs`)

All run the blocking PeerTube calls on `tokio::task::spawn_blocking` (as the
existing `get_remote_library` does) and 400 on a catacomb remote.

| Method / path | Returns |
|---|---|
| `GET /api/remotes/:id/channels` | `[{handle, display_name, video_count, avatar_url}]` |
| `GET /api/remotes/:id/channels/:handle/videos?page=N` | `[{id, title, channel, thumb_url, duration_secs}]` (page 24; `N` defaults 0) |
| `GET /api/remotes/:id/videos/:uuid/media` | `{url}` (200) or `204 No Content` when HLS-only |
| `POST /api/remotes/:id/archive` `{uuid}` | `202 "ok"` after `downloader.start(watch_url, …)`; 404 unknown remote, 400 catacomb remote |

`:handle` may contain `@host` for a federated channel, so it is a path segment
that is percent-decoded by axum; the handler passes it verbatim to
`channel_videos`. The archive handler resolves `watch_url(uuid)`, builds the
`UrlInfo` with the existing synchronous `classify_url(&url)` (no network probe;
`info.platform == Other` for a PeerTube URL), and calls the shared
`Downloader::start`. It mirrors `post_download`'s shape: `start` returns `()`,
so the response is `202 "ok"` and the job then appears in the normal downloads
panel via the existing progress stream (no job id is returned).

## Web UI (`src/web_ui/index.html`)

A new **PeerTube browse mode**, distinct from the existing flat-library
`remoteMode` (the PeerTube view is a two-level nav, not the reused grid):

- **Enter:** clicking a remote whose `kind === 'peertube'` in the sidebar enters
  PeerTube mode and calls `GET …/channels`. (A catacomb remote still calls
  `enterRemote`.) State: `ptRemoteId`, `ptChannel`, `ptPage`, `ptVideos`.
- **Channel list:** rows with avatar, display name, and video count. Click →
  load that channel's videos.
- **Video grid:** cards (thumbnail, title, duration) with **▶ Play** and
  **⬇ Archive**, plus a **[Load more]** button that fetches the next page and
  appends (hidden when a page returns < 24). 
- **Play:** `GET …/videos/:uuid/media`; on 200 open the existing custom player
  (`playVideo`) with the returned URL; on 204 the Play button is disabled with an
  "HLS-only — archive to watch" note.
- **Archive:** `POST …/archive {uuid}`; toast on success; the job shows up in the
  normal downloads panel via the existing progress stream.
- **Back navigation:** video grid → channel list → "Back to my library"
  (`exitRemote`-style reset). PeerTube mode reuses the sidebar remotes list for
  peer switching.

Sidebar: the existing `🌐 Remotes` block already lists every peer with
`enterRemote(id)`. Dispatch there on `r.kind` — peertube → `enterPeertube(id)`.

## Desktop UI (`src/app.rs`)

`remotes_screen` dispatches on `RemoteClientKind::kind()`. The Peertube arm
replaces the phase-2 stopgap with the same two-level nav in egui:

- New `App` state: `pt_channels: Option<Vec<RemoteChannelInfo>>`,
  `pt_selected_channel: Option<String>`, `pt_videos: Vec<RemoteVideo>`,
  `pt_page: usize`, and mpsc receivers for the background channel/video fetches
  (`pt_channels_rx`, `pt_videos_rx`) drained in `update()` — mirroring the
  existing `remote_rx` pattern (fetch on a thread, `request_repaint`).
- Channel list (selectable rows) → click loads page 0 of that channel's videos;
  a **Load more** button appends the next page.
- Per video row: **Play** — resolve `video_media` on a thread, then hand the URL
  to the existing `play_remote_url` (mpv); greyed when the resolve returns `None`.
  **Archive** — `self.downloader.start(watch_url, …)` (probe → `Other`).
- Status/errors surface on the existing `remote_status` line.

## Archive action (shared)

Both UIs route through the same `Downloader::start` a manual download uses, so
auto-retry, post-download transcode, and the hang watchdog all apply unchanged.
The archived video lands in `other/<uploader>/` and appears in the local library
after the next scan (the download pipeline already triggers this). No new
download settings, no per-remote archive options.

## HLS-only & error handling

- `video_media == None` (HLS-only): **Play disabled**, Archive still works
  (yt-dlp downloads HLS fine). The disabled Play carries an explanatory tooltip.
- Network / auth / not-found failures from the PeerTube client surface as an
  inline error line in the browse view (web: a status/toast; desktop:
  `remote_status`) — never a panic or a stuck "loading…".
- An empty channel (zero videos) shows an empty-state row, not a blank grid.
- Calling a new endpoint on a catacomb remote → `400` with a clear message; the
  UIs never do this (they dispatch on kind) but the guard documents intent and
  protects the API.

## Testing

- **Unit (web.rs):** the kind-guard — a new endpoint handler (or the
  `RemoteClientKind` passthrough) returns `Err`/400 for a `Catacomb` arm. An
  archive-path unit asserting a given `uuid` maps to the expected `watch_url`
  before hand-off.
- **Integration (`tests/api.rs`):** `PUT` a peertube remote, then `GET
  …/channels` against an unreachable host — asserts the route exists and returns
  a client/gateway error (not 404-route-missing), and that `GET …/channels` on a
  *catacomb* remote returns 400. No network required (unreachable host → fast
  connection error); skip-if-curl-absent like the file's other tests.
- **Mapping:** already fixture-tested in phase 1 (`map_channel`, `map_video`,
  `pick_media`) — no change.
- **Manual:** against `https://framatube.org` (public instance): browse
  channels, open one, Load more, play a direct-MP4 video inline (web) / via mpv
  (desktop), confirm an HLS-only video disables Play, archive one video and
  confirm it lands under `Other` and appears after a rescan.

## Out of scope (possible later)

- Bulk / whole-channel archive.
- A dedicated `Platform::Peertube` library section.
- Search within a peer, subscriptions/feeds, comments, or write actions.
- Caching channel/video lists (each browse is a live fetch).
