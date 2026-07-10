# PeerTube client + multi-kind config (backend foundation)

**Date:** 2026-07-10
**Status:** Approved design, ready for implementation plan
**Scope:** Backend only — no UI. Sub-project 1 of the PeerTube federation work.

## Context

This is the first of three phased sub-projects that together let catacomb
federate with PeerTube instances/channels in addition to catacomb peers:

1. **PeerTube client + multi-kind config (this spec)** — the backend foundation:
   a `RemoteKind`, config fields, and a `PeerTubeClient` that lists channels and
   fetches paginated videos, mapped into the existing remote types. Headless,
   unit-tested.
2. **Kind-aware remote editor** — the editor from
   `2026-07-10-federation-remote-editor-design.md`, extended with a kind
   selector + username field, both UIs.
3. **PeerTube browse UI + archive action** — two-level lazy navigation (list
   channels → click → load that channel's videos) + a per-video "Archive"
   button wiring the PeerTube watch URL into the downloader, both UIs.

Each phase ships independently. This spec covers phase 1 only.

## Problem

catacomb federation (`src/remote.rs`) can only browse *other catacomb
instances*. PeerTube is a large federated video network catacomb cannot
currently browse read-only. yt-dlp already downloads PeerTube URLs, but there is
no way to browse a PeerTube instance/channel inside catacomb. Phase 1 builds the
backend client and the config model the later UI phases consume.

## Goal

A blocking `PeerTubeClient` that, given a `RemoteSection` of kind `peertube`,
can: authenticate (OAuth2, if credentials are set), list the target's video
channels, fetch a channel's videos paginated, resolve a video's playable media
URL on demand, and produce the canonical watch URL for archiving — all mapped
into the existing `remote` types. Plus the non-breaking config model.

## Non-goals (this phase)

- No UI (editor and browse are phases 2 and 3).
- No `RemoteSource` trait / catacomb-PeerTube polymorphism yet — introduced in
  phase 2 when a shared UI consumes both kinds (YAGNI until a second consumer).
- No changes to the existing catacomb `RemoteClient` behavior.
- No live-network integration test (needs a real instance; covered by
  fixture-based unit tests + manual verification).

## Config model

`src/config.rs` — additive, `#[serde(default)]` so existing configs are
untouched:

```rust
/// Which kind of peer a `[[remote]]` is. Defaults to a catacomb peer so
/// pre-existing config.toml entries keep working unchanged.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RemoteKind {
    #[default]
    Catacomb,
    Peertube,
}

pub struct RemoteSection {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub kind: RemoteKind,          // NEW
    #[serde(default)]
    pub username: Option<String>,  // NEW — PeerTube OAuth username
    #[serde(default)]
    pub password: Option<String>,
}
```

A catacomb entry (no `kind`) deserializes as `RemoteKind::Catacomb`,
`username = None`. No migration needed.

## `PeerTubeClient` (`src/peertube.rs`)

New module. Blocking `reqwest` client with a cookie/connection pool, mirroring
`RemoteClient`'s construction. Holds the parsed API base, the target, optional
credentials, and a `Mutex<Option<OAuthTokens>>` cache.

```rust
pub struct PeerTubeClient {
    pub name: String,
    api_base: String,          // scheme://host (no trailing slash)
    target: Target,            // Instance | Account(String) | Channel(String)
    username: Option<String>,
    password: Option<String>,
    client: reqwest::blocking::Client,
    tokens: std::sync::Mutex<Option<OAuthTokens>>,
}

enum Target { Instance, Account(String), Channel(String) }

struct OAuthTokens { access: String, refresh: String }

pub struct RemoteChannelInfo {
    pub handle: String,        // e.g. "blender_open_movies" or "foo@other.tld"
    pub display_name: String,
    pub video_count: Option<u64>,
    pub avatar_url: Option<String>,
}
// Videos map into the existing remote::RemoteVideo.
```

### URL / handle parsing (`fn parse_target(url) -> (api_base, Target)`)

From the remote's `url`:
- `api_base` = `scheme://host[:port]`.
- Path `/(c|video-channels)/{handle}` → `Target::Channel(handle)`.
- Path `/(a|accounts)/{name}` → `Target::Account(name)`.
- Bare host / `/` → `Target::Instance`.

`handle`/`name` is the last non-empty path segment (may contain `@host` for a
federated channel, kept verbatim).

### OAuth2 (only when both `username` and non-empty `password` are set)

Anonymous (public) mode otherwise — plain GETs, no `Authorization`.

1. `GET {api_base}/api/v1/oauth-clients/local` → `{ client_id, client_secret }`.
2. `POST {api_base}/api/v1/users/token` (form-encoded): `client_id`,
   `client_secret`, `grant_type=password`, `username`, `password` →
   `{ access_token, refresh_token, expires_in }`. Cache both tokens.
3. Authenticated requests send `Authorization: Bearer {access}`.
4. On a `401`, refresh via `grant_type=refresh_token`; if refresh fails, redo the
   password grant once. A second failure surfaces as an error.

`authed_get(path)` centralises this (parallels `RemoteClient::authed_get`).

### Methods

```rust
pub fn list_channels(&self) -> Result<Vec<RemoteChannelInfo>, String>;
pub fn channel_videos(&self, handle: &str, page: usize) -> Result<Vec<crate::remote::RemoteVideo>, String>;
pub fn video_media(&self, uuid: &str) -> Result<Option<String>, String>;
pub fn watch_url(&self, uuid: &str) -> String;
```

- **`list_channels`**:
  - `Instance` → `GET /api/v1/video-channels?start=0&count=100` (paginate until
    `total` consumed or a sane cap).
  - `Account(n)` → `GET /api/v1/accounts/{n}/video-channels`.
  - `Channel(h)` → one `GET /api/v1/video-channels/{h}` mapped to a single
    `RemoteChannelInfo`.
  - Map each PeerTube channel object → `RemoteChannelInfo { handle: name (+@host
    if remote), display_name: displayName, video_count: videosCount, avatar_url:
    api_base + avatars[…].path }`.
- **`channel_videos(handle, page)`**: `GET
  /api/v1/video-channels/{handle}/videos?start={page*24}&count=24&sort=-publishedAt`.
  Each list object → `RemoteVideo { id: uuid, title: name, channel: <handle's
  display>, video_url: None, thumb_url: Some(api_base + thumbnailPath),
  duration_secs: Some(duration) }`. `video_url` is `None` because list objects
  omit `files`; it is resolved on demand by `video_media`.
- **`video_media(uuid)`**: `GET /api/v1/videos/{uuid}` → choose a direct MP4 from
  `files[].fileUrl` (prefer the highest resolution ≤ 1080p; any if none match).
  For a private video, append `?videoFileToken={t}` obtained from
  `POST /api/v1/videos/{uuid}/token` (only when authenticated). HLS-only (empty
  `files`, non-empty `streamingPlaylists`) → `Ok(None)`.
- **`watch_url(uuid)`**: `format!("{api_base}/w/{uuid}")`.

### Media / playback constraint

Direct-MP4 (`files[].fileUrl`) videos stream inline. HLS-only videos return
`video_url = None`; phase 3's UI will show them as browse-only (no inline
player) while the Archive action still works (yt-dlp handles HLS). Documenting
this limitation here so phase 3 doesn't treat it as a bug.

## Error handling

Every method returns `Result<_, String>`. Network, JSON-parse, non-2xx, and auth
failures become descriptive error strings (`"peertube {name}: HTTP 404"`,
`"oauth token: …"`). `PeerTubeClient::new` is total (parsing a malformed URL
still constructs a client whose first request fails with a clear error) — no
panics. A missing/renamed JSON field maps to `None`/skips the item rather than
erroring the whole list.

## Testing (headless, fixture-based)

Unit tests in `src/peertube.rs`:
- **`parse_target`**: instance root, `/c/{h}`, `/video-channels/{h}`, `/a/{n}`,
  `/accounts/{n}`, and a federated `/c/foo@other.tld` handle → correct
  `(api_base, Target)`.
- **channel mapping**: fixture `/api/v1/video-channels` JSON → `RemoteChannelInfo`
  (display name, count, absolutified avatar).
- **video mapping**: fixture channel-videos JSON → `RemoteVideo` (uuid, title,
  duration, `thumb_url` absolutified, `video_url == None`).
- **media pick**: fixture video-detail JSON with several `files[]` → the chosen
  MP4 `fileUrl`; HLS-only fixture (`files: []`) → `None`.
- **oauth parse**: fixture token response → `OAuthTokens { access, refresh }`.

Pure mapping/parse fns take `&serde_json::Value` (or `&str`) so tests need no
network. The actual HTTP round-trips are verified manually against a real
public instance (e.g. framatube.org) during phase 3 bring-up.

## What phase 2 / 3 consume

- Phase 2 (editor): the `RemoteKind` + `username` config fields, and a
  `RemoteSource` trait extracted then so the editor and browse treat both kinds
  uniformly.
- Phase 3 (browse UI): `list_channels` → channel list; `channel_videos` →
  per-channel grid (lazy on click, paginated); `video_media` → inline play;
  `watch_url` → the Archive action's downloader input.
```
