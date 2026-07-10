# PeerTube Client + Multi-Kind Config — Implementation Plan (Phase 1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `RemoteKind` config model and a headless blocking `PeerTubeClient` that lists a PeerTube target's channels, fetches a channel's videos paginated, resolves a video's playable MP4 on demand, and builds its watch URL — all mapped into the existing `remote` types.

**Architecture:** A new `src/peertube.rs` module holds pure mapping/parse functions (URL→target, JSON→`RemoteChannelInfo`/`RemoteVideo`, media pick, OAuth token parse) that are unit-tested against fixtures, plus a thin blocking-`reqwest` `PeerTubeClient` that wires those pure fns to PeerTube's public REST API (with optional OAuth2). No UI (phases 2–3).

**Tech Stack:** Rust, `reqwest` (blocking, json, cookies — already a dep), `serde`/`serde_json`.

## Global Constraints

- Backend only — no UI, no changes to the existing `remote::RemoteClient` behavior.
- Config additions are `#[serde(default)]` and non-breaking: an existing `[[remote]]` with no `kind` deserializes as `RemoteKind::Catacomb`, `username = None`.
- Every `PeerTubeClient` method returns `Result<_, String>`; no panics. `new` is total.
- Pure mapping/parse fns take `&serde_json::Value` (or `&str`) so tests need no network. HTTP round-trips are verified manually against a real instance during phase 3.
- No new `RemoteSource` trait this phase (YAGNI until phase 2's shared UI).
- `.form()` and `.json()` on the blocking client are available (reqwest features `blocking`, `json` present). Cookies enabled.
- Commits are SSH-signed: `export SSH_AUTH_SOCK=/tmp/luna-ssh-agent.sock`. End messages with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.
- Spec: `docs/superpowers/specs/2026-07-10-peertube-client-backend-design.md`.

---

### Task 1: Multi-kind config model

**Files:**
- Modify: `src/config.rs` — add `RemoteKind` enum; add `kind` + `username` fields to `RemoteSection`.

**Interfaces:**
- Produces: `pub enum RemoteKind { Catacomb, Peertube }` (`Default = Catacomb`); `RemoteSection.kind: RemoteKind`, `RemoteSection.username: Option<String>`.

- [ ] **Step 1: Write the failing test**

Add to `src/config.rs` in a `#[cfg(test)] mod tests` block (create it at end of file with `use super::*;` if absent):

```rust
#[test]
fn remote_kind_defaults_to_catacomb() {
    // A legacy [[remote]] with no `kind`/`username` still parses. `[backup]`
    // has no serde default (directory is required), so include a minimal one.
    let toml = r#"
        [backup]
        directory = "/tmp/lib"

        [[remote]]
        name = "peer"
        url = "http://peer:8081"
    "#;
    let cfg: Config = toml::from_str(toml).unwrap();
    assert_eq!(cfg.remotes.len(), 1);
    assert_eq!(cfg.remotes[0].kind, RemoteKind::Catacomb);
    assert!(cfg.remotes[0].username.is_none());
}

#[test]
fn remote_kind_parses_peertube_and_username() {
    let toml = r#"
        [backup]
        directory = "/tmp/lib"

        [[remote]]
        name = "frama"
        url = "https://framatube.org"
        kind = "peertube"
        username = "alice"
        password = "secret"
    "#;
    let cfg: Config = toml::from_str(toml).unwrap();
    assert_eq!(cfg.remotes[0].kind, RemoteKind::Peertube);
    assert_eq!(cfg.remotes[0].username.as_deref(), Some("alice"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --release remote_kind 2>&1 | grep -E "error\[|cannot find|RemoteKind"`
Expected: FAIL — `RemoteKind` and the `kind`/`username` fields don't exist.

- [ ] **Step 3: Add the enum and fields**

In `src/config.rs`, add the enum above `RemoteSection`:

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
```

Add the two fields to `RemoteSection` (after `url`, before `password`):

```rust
    /// Peer kind — catacomb federation peer (default) or a PeerTube target.
    #[serde(default)]
    pub kind: RemoteKind,
    /// Username for PeerTube OAuth2. `None` for catacomb peers / public
    /// PeerTube.
    #[serde(default)]
    pub username: Option<String>,
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --release remote_kind 2>&1 | grep "test result"`
Expected: `test result: ok. 2 passed`.

- [ ] **Step 5: Commit**

```bash
export SSH_AUTH_SOCK=/tmp/luna-ssh-agent.sock
git add src/config.rs
git commit -m "feat(config): RemoteKind + username for PeerTube remotes

Non-breaking: existing [[remote]] entries default to kind=catacomb.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: peertube module — target parsing + client skeleton

**Files:**
- Create: `src/peertube.rs`
- Modify: `src/main.rs` — add `mod peertube;` (between `mod maintenance;` and `mod platform;`).

**Interfaces:**
- Consumes: `crate::config::RemoteSection` (Task 1).
- Produces:
  - `pub struct PeerTubeClient { pub name: String, api_base: String, target: Target, username: Option<String>, password: Option<String>, client: reqwest::blocking::Client, tokens: std::sync::Mutex<Option<OAuthTokens>> }`
  - `PeerTubeClient::new(cfg: &RemoteSection) -> Self`
  - `pub fn watch_url(&self, uuid: &str) -> String`
  - internal `enum Target { Instance, Account(String), Channel(String) }`, `struct OAuthTokens { access: String, refresh: String }`, `fn parse_target(url: &str) -> (String, Target)`.

- [ ] **Step 1: Create the module with parsing + skeleton**

Create `src/peertube.rs`:

```rust
//! Read-only PeerTube client: lists a target's channels and their videos over
//! PeerTube's public REST API (with optional OAuth2), mapped into the existing
//! `crate::remote` types. Blocking, like `crate::remote::RemoteClient`.
//! Phase 1 of the PeerTube federation work (backend only).

use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;

use crate::config::RemoteSection;
use crate::remote::RemoteVideo;

/// What a PeerTube remote points at.
enum Target {
    Instance,
    Account(String),
    Channel(String),
}

struct OAuthTokens {
    access: String,
    refresh: String,
}

/// One channel in a PeerTube target's channel list.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteChannelInfo {
    pub handle: String,
    pub display_name: String,
    pub video_count: Option<u64>,
    pub avatar_url: Option<String>,
}

pub struct PeerTubeClient {
    pub name: String,
    api_base: String,
    target: Target,
    username: Option<String>,
    password: Option<String>,
    client: reqwest::blocking::Client,
    tokens: Mutex<Option<OAuthTokens>>,
}

/// Derive the API base (`scheme://host[:port]`) and the target from a remote URL.
/// `/(c|video-channels)/{h}` → Channel; `/(a|accounts)/{n}` → Account; else
/// Instance. The handle/name is the segment after the marker, kept verbatim
/// (may include `@host` for a federated channel).
fn parse_target(url: &str) -> (String, Target) {
    let trimmed = url.trim().trim_end_matches('/');
    // Split scheme://host from the path.
    let (scheme_host, path) = match trimmed.find("://") {
        Some(i) => {
            let after = &trimmed[i + 3..];
            match after.find('/') {
                Some(j) => (&trimmed[..i + 3 + j], &after[j..]),
                None => (trimmed, ""),
            }
        }
        None => (trimmed, ""),
    };
    let api_base = scheme_host.to_string();
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let target = match segs.as_slice() {
        [marker, handle, ..] if *marker == "c" || *marker == "video-channels" => {
            Target::Channel((*handle).to_string())
        }
        [marker, name, ..] if *marker == "a" || *marker == "accounts" => {
            Target::Account((*name).to_string())
        }
        _ => Target::Instance,
    };
    (api_base, target)
}

impl PeerTubeClient {
    pub fn new(cfg: &RemoteSection) -> Self {
        let (api_base, target) = parse_target(&cfg.url);
        let client = reqwest::blocking::Client::builder()
            .cookie_store(true)
            .timeout(Duration::from_secs(30))
            .user_agent("catacomb-peertube")
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        PeerTubeClient {
            name: cfg.name.clone(),
            api_base,
            target,
            username: cfg.username.clone().filter(|u| !u.is_empty()),
            password: cfg.password.clone().filter(|p| !p.is_empty()),
            client,
            tokens: Mutex::new(None),
        }
    }

    /// Canonical watch URL for a video, handed to the downloader (phase 3).
    pub fn watch_url(&self, uuid: &str) -> String {
        format!("{}/w/{}", self.api_base, uuid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target_str(t: &Target) -> String {
        match t {
            Target::Instance => "instance".into(),
            Target::Account(n) => format!("account:{n}"),
            Target::Channel(h) => format!("channel:{h}"),
        }
    }

    #[test]
    fn parse_target_variants() {
        let cases = [
            ("https://framatube.org", "https://framatube.org", "instance"),
            ("https://framatube.org/", "https://framatube.org", "instance"),
            ("https://framatube.org/c/blender", "https://framatube.org", "channel:blender"),
            ("https://framatube.org/video-channels/blender", "https://framatube.org", "channel:blender"),
            ("https://framatube.org/a/framasoft", "https://framatube.org", "account:framasoft"),
            ("https://framatube.org/accounts/framasoft", "https://framatube.org", "account:framasoft"),
            ("https://framatube.org/c/foo@other.tld", "https://framatube.org", "channel:foo@other.tld"),
            ("http://peer:9000/c/x", "http://peer:9000", "channel:x"),
        ];
        for (url, base, tgt) in cases {
            let (b, t) = parse_target(url);
            assert_eq!(b, base, "base for {url}");
            assert_eq!(target_str(&t), tgt, "target for {url}");
        }
    }

    #[test]
    fn watch_url_built_from_base() {
        let cfg = RemoteSection {
            name: "f".into(),
            url: "https://framatube.org/c/blender".into(),
            kind: crate::config::RemoteKind::Peertube,
            username: None,
            password: None,
        };
        let c = PeerTubeClient::new(&cfg);
        assert_eq!(c.watch_url("abc-123"), "https://framatube.org/w/abc-123");
    }
}
```

- [ ] **Step 2: Register the module**

In `src/main.rs`, add `mod peertube;` between `mod maintenance;` and `mod platform;`.

- [ ] **Step 3: Run the tests to verify they pass**

Run: `cargo test --release peertube 2>&1 | grep -E "^error|test result"`
Expected: compiles; `test result: ok. 2 passed`. (A `dead_code` warning for the not-yet-used `target`/`tokens`/`OAuthTokens` fields is fine — they're used in Task 4.)

- [ ] **Step 4: Commit**

```bash
export SSH_AUTH_SOCK=/tmp/luna-ssh-agent.sock
git add src/peertube.rs src/main.rs
git commit -m "feat(peertube): module skeleton + URL/target parsing

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Pure JSON mapping + parse functions

**Files:**
- Modify: `src/peertube.rs` — add `map_channel`, `map_video`, `pick_media`, `parse_tokens`; tests.

**Interfaces:**
- Consumes: `RemoteChannelInfo`, `OAuthTokens`, `RemoteVideo` (Tasks 2 / `remote.rs`).
- Produces (module-private):
  - `fn map_channel(v: &Value, api_base: &str) -> Option<RemoteChannelInfo>`
  - `fn map_video(v: &Value, api_base: &str, channel: &str) -> RemoteVideo`
  - `fn pick_media(detail: &Value) -> Option<String>`
  - `fn parse_tokens(v: &Value) -> Option<OAuthTokens>`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `src/peertube.rs`:

```rust
use serde_json::json;

#[test]
fn map_channel_maps_fields_and_absolutifies_avatar() {
    let v = json!({
        "name": "blender", "displayName": "Blender Open Movies",
        "videosCount": 12, "avatars": [{ "path": "/lazy-static/avatars/x.png" }]
    });
    let c = map_channel(&v, "https://framatube.org").unwrap();
    assert_eq!(c.handle, "blender");
    assert_eq!(c.display_name, "Blender Open Movies");
    assert_eq!(c.video_count, Some(12));
    assert_eq!(c.avatar_url.as_deref(), Some("https://framatube.org/lazy-static/avatars/x.png"));
}

#[test]
fn map_channel_federated_handle_includes_host() {
    let v = json!({ "name": "foo", "displayName": "Foo", "host": "other.tld" });
    let c = map_channel(&v, "https://framatube.org").unwrap();
    assert_eq!(c.handle, "foo@other.tld");
}

#[test]
fn map_video_maps_and_absolutifies_thumb() {
    let v = json!({
        "uuid": "abc-123", "name": "My Vid", "duration": 61,
        "thumbnailPath": "/lazy-static/thumbnails/abc.jpg"
    });
    let vid = map_video(&v, "https://framatube.org", "Blender");
    assert_eq!(vid.id, "abc-123");
    assert_eq!(vid.title, "My Vid");
    assert_eq!(vid.channel, "Blender");
    assert_eq!(vid.duration_secs, Some(61.0));
    assert_eq!(vid.thumb_url.as_deref(), Some("https://framatube.org/lazy-static/thumbnails/abc.jpg"));
    assert!(vid.video_url.is_none()); // resolved later via video_media
}

#[test]
fn pick_media_prefers_direct_mp4() {
    let detail = json!({
        "files": [
            { "resolution": { "id": 480 }, "fileUrl": "https://f/480.mp4" },
            { "resolution": { "id": 1080 }, "fileUrl": "https://f/1080.mp4" }
        ]
    });
    assert_eq!(pick_media(&detail).as_deref(), Some("https://f/1080.mp4"));
}

#[test]
fn pick_media_hls_only_is_none() {
    let detail = json!({ "files": [], "streamingPlaylists": [{ "playlistUrl": "https://f/master.m3u8" }] });
    assert_eq!(pick_media(&detail), None);
}

#[test]
fn parse_tokens_reads_access_and_refresh() {
    let v = json!({ "access_token": "AAA", "token_type": "Bearer", "expires_in": 3600, "refresh_token": "RRR" });
    let t = parse_tokens(&v).unwrap();
    assert_eq!(t.access, "AAA");
    assert_eq!(t.refresh, "RRR");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --release peertube 2>&1 | grep -E "cannot find|test result"`
Expected: FAIL — `map_channel`/`map_video`/`pick_media`/`parse_tokens` not found.

- [ ] **Step 3: Implement the pure functions**

Add to `src/peertube.rs` (module scope, not inside `impl`):

```rust
/// Absolutify a PeerTube relative path (`/lazy-static/…`) against the API base.
fn absolutify(api_base: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        path.to_string()
    } else {
        format!("{api_base}{path}")
    }
}

fn map_channel(v: &Value, api_base: &str) -> Option<RemoteChannelInfo> {
    let name = v.get("name").and_then(Value::as_str)?;
    let handle = match v.get("host").and_then(Value::as_str) {
        Some(host) if !host.is_empty() => format!("{name}@{host}"),
        _ => name.to_string(),
    };
    let display_name = v
        .get("displayName")
        .and_then(Value::as_str)
        .unwrap_or(name)
        .to_string();
    let video_count = v.get("videosCount").and_then(Value::as_u64);
    // Newer PeerTube: `avatars: [{path}]`; older: `avatar: {path}`.
    let avatar_path = v
        .get("avatars")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|a| a.get("path"))
        .or_else(|| v.get("avatar").and_then(|a| a.get("path")))
        .and_then(Value::as_str);
    let avatar_url = avatar_path.map(|p| absolutify(api_base, p));
    Some(RemoteChannelInfo { handle, display_name, video_count, avatar_url })
}

fn map_video(v: &Value, api_base: &str, channel: &str) -> RemoteVideo {
    let id = v.get("uuid").and_then(Value::as_str).unwrap_or("").to_string();
    let title = v
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("(untitled)")
        .to_string();
    let duration_secs = v.get("duration").and_then(Value::as_f64);
    let thumb_url = v
        .get("thumbnailPath")
        .and_then(Value::as_str)
        .map(|p| absolutify(api_base, p));
    RemoteVideo {
        id,
        title,
        channel: channel.to_string(),
        video_url: None,
        thumb_url,
        duration_secs,
    }
}

/// Pick a directly-playable MP4 from a video detail object. Prefers the highest
/// resolution ≤ 1080; falls back to the highest available. HLS-only (empty
/// `files`) → None.
fn pick_media(detail: &Value) -> Option<String> {
    let files = detail.get("files").and_then(Value::as_array)?;
    if files.is_empty() {
        return None;
    }
    let res = |f: &Value| f.get("resolution").and_then(|r| r.get("id")).and_then(Value::as_u64).unwrap_or(0);
    // Highest res ≤ 1080, else highest overall.
    let best = files
        .iter()
        .filter(|f| res(f) <= 1080)
        .max_by_key(|f| res(f))
        .or_else(|| files.iter().max_by_key(|f| res(f)))?;
    best.get("fileUrl").and_then(Value::as_str).map(String::from)
}

fn parse_tokens(v: &Value) -> Option<OAuthTokens> {
    let access = v.get("access_token").and_then(Value::as_str)?.to_string();
    let refresh = v.get("refresh_token").and_then(Value::as_str).unwrap_or("").to_string();
    Some(OAuthTokens { access, refresh })
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --release peertube 2>&1 | grep "test result"`
Expected: `test result: ok. 8 passed` (2 from Task 2 + 6 here).

- [ ] **Step 5: Commit**

```bash
export SSH_AUTH_SOCK=/tmp/luna-ssh-agent.sock
git add src/peertube.rs
git commit -m "feat(peertube): pure JSON mapping (channels, videos, media, tokens)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: HTTP methods (OAuth + list/videos/media)

Wires the pure fns to PeerTube's REST API. Not unit-tested (network); verified by build + an optional manual smoke against a public instance. Correctness rests on Task 3's tested mapping fns.

**Files:**
- Modify: `src/peertube.rs` — add OAuth (`ensure_token`/`authed_get`) and `list_channels`, `channel_videos`, `video_media`.

**Interfaces:**
- Consumes: Task 3 pure fns; Task 2 `Target`/`OAuthTokens`/`api_base`/`client`.
- Produces:
  - `pub fn list_channels(&self) -> Result<Vec<RemoteChannelInfo>, String>`
  - `pub fn channel_videos(&self, handle: &str, page: usize) -> Result<Vec<RemoteVideo>, String>`
  - `pub fn video_media(&self, uuid: &str) -> Result<Option<String>, String>`

- [ ] **Step 1: Implement OAuth + authed_get**

Add to `impl PeerTubeClient` in `src/peertube.rs`:

```rust
/// Obtain (and cache) an access token via the OAuth2 password grant. No-op /
/// returns None when no credentials are configured (public browsing).
fn ensure_token(&self) -> Result<Option<String>, String> {
    let (Some(user), Some(pass)) = (&self.username, &self.password) else {
        return Ok(None);
    };
    if let Some(t) = self.tokens.lock().unwrap().as_ref() {
        return Ok(Some(t.access.clone()));
    }
    // 1. oauth client id/secret.
    let oc: Value = self
        .client
        .get(format!("{}/api/v1/oauth-clients/local", self.api_base))
        .send()
        .map_err(|e| format!("peertube {}: oauth-clients: {e}", self.name))?
        .json()
        .map_err(|e| format!("peertube {}: oauth-clients parse: {e}", self.name))?;
    let client_id = oc.get("client_id").and_then(Value::as_str).unwrap_or("");
    let client_secret = oc.get("client_secret").and_then(Value::as_str).unwrap_or("");
    // 2. password grant.
    let resp = self
        .client
        .post(format!("{}/api/v1/users/token", self.api_base))
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("grant_type", "password"),
            ("username", user.as_str()),
            ("password", pass.as_str()),
        ])
        .send()
        .map_err(|e| format!("peertube {}: token: {e}", self.name))?;
    if !resp.status().is_success() {
        return Err(format!("peertube {}: token HTTP {}", self.name, resp.status().as_u16()));
    }
    let v: Value = resp.json().map_err(|e| format!("peertube {}: token parse: {e}", self.name))?;
    let toks = parse_tokens(&v).ok_or_else(|| format!("peertube {}: token missing", self.name))?;
    let access = toks.access.clone();
    *self.tokens.lock().unwrap() = Some(toks);
    Ok(Some(access))
}

/// GET an API path, adding a Bearer token when credentials are set, retrying
/// once after clearing a stale token on 401.
fn authed_get(&self, path: &str) -> Result<Value, String> {
    let url = format!("{}{}", self.api_base, path);
    let send = |token: &Option<String>| {
        let mut req = self.client.get(&url);
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        req.send()
    };
    let token = self.ensure_token()?;
    let resp = send(&token).map_err(|e| format!("peertube {}: {path}: {e}", self.name))?;
    let resp = if resp.status().as_u16() == 401 && token.is_some() {
        *self.tokens.lock().unwrap() = None; // force re-grant
        let fresh = self.ensure_token()?;
        send(&fresh).map_err(|e| format!("peertube {}: {path}: {e}", self.name))?
    } else {
        resp
    };
    if !resp.status().is_success() {
        return Err(format!("peertube {}: {path}: HTTP {}", self.name, resp.status().as_u16()));
    }
    resp.json().map_err(|e| format!("peertube {}: {path}: parse {e}", self.name))
}
```

- [ ] **Step 2: Implement list_channels / channel_videos / video_media**

Add to the same `impl PeerTubeClient`:

```rust
/// List the target's video channels.
pub fn list_channels(&self) -> Result<Vec<RemoteChannelInfo>, String> {
    let path = match &self.target {
        Target::Instance => "/api/v1/video-channels?start=0&count=100".to_string(),
        Target::Account(n) => format!("/api/v1/accounts/{n}/video-channels?start=0&count=100"),
        Target::Channel(h) => {
            // Single channel — fetch its object directly and map it alone.
            let v = self.authed_get(&format!("/api/v1/video-channels/{h}"))?;
            return Ok(map_channel(&v, &self.api_base).into_iter().collect());
        }
    };
    let v = self.authed_get(&path)?;
    let data = v.get("data").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(data.iter().filter_map(|c| map_channel(c, &self.api_base)).collect())
}

/// Fetch one page (24) of a channel's videos, newest first.
pub fn channel_videos(&self, handle: &str, page: usize) -> Result<Vec<RemoteVideo>, String> {
    let start = page * 24;
    let path = format!(
        "/api/v1/video-channels/{handle}/videos?start={start}&count=24&sort=-publishedAt"
    );
    let v = self.authed_get(&path)?;
    let data = v.get("data").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(data
        .iter()
        .map(|vid| {
            let channel = vid
                .get("channel")
                .and_then(|c| c.get("displayName"))
                .and_then(Value::as_str)
                .unwrap_or(handle)
                .to_string();
            map_video(vid, &self.api_base, &channel)
        })
        .collect())
}

/// Resolve a video's directly-playable MP4 URL (None if HLS-only).
pub fn video_media(&self, uuid: &str) -> Result<Option<String>, String> {
    let v = self.authed_get(&format!("/api/v1/videos/{uuid}"))?;
    Ok(pick_media(&v))
}
```

- [ ] **Step 3: Verify it compiles clean**

Run: `cargo build --release 2>&1 | grep -E "^error|^warning: unused" | head`
Expected: no `error` lines. The `dead_code` warnings from Task 2 (`target`, `tokens`, `OAuthTokens`) are now gone (all used).

- [ ] **Step 4: Full test suite**

Run: `cargo test --release 2>&1 | grep "test result"`
Expected: all `ok`, 0 failed (unit count +8 vs the pre-Task-1 baseline).

- [ ] **Step 5 (optional): Manual smoke against a public instance**

If online, in a throwaway `src/main.rs`-style scratch or a `#[ignore]` test, construct a client for `https://framatube.org` and confirm `list_channels()` returns entries and `channel_videos(<handle>, 0)` returns videos with absolutified thumbs. This is bring-up confidence only; not required to land the phase.

- [ ] **Step 6: Commit**

```bash
export SSH_AUTH_SOCK=/tmp/luna-ssh-agent.sock
git add src/peertube.rs
git commit -m "feat(peertube): OAuth2 + list channels / videos / media over REST

Completes the phase-1 backend client: password-grant OAuth (public browsing
when unset), list_channels, paginated channel_videos, and on-demand video_media
(direct MP4, None for HLS-only). Wires the tested pure mapping fns to the API.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- Config model (`RemoteKind` + `kind`/`username`, non-breaking) → Task 1. ✓
- URL/handle parsing (instance / `/c/` / `/video-channels/` / `/a/` / `/accounts/` / federated `@host`) → Task 2 `parse_target` + test. ✓
- OAuth2 (client id/secret → password grant → Bearer → 401 re-grant), anonymous when unset → Task 4 `ensure_token`/`authed_get`. ✓
- `list_channels` (instance/account/single-channel) → Task 4. ✓
- `channel_videos(handle, page)` paginated, newest-first, mapped → Task 4 + Task 3 `map_video`. ✓
- `video_media(uuid)` direct MP4 / None for HLS → Task 4 + Task 3 `pick_media`. ✓
- `watch_url(uuid)` → Task 2. ✓
- Error handling: every method `Result<_, String>`, no panics → Tasks 2/4. ✓
- Testing: parse_target, channel/video/media/token mapping fixtures → Tasks 2/3. ✓
- Non-goals honored: no UI, no `RemoteSource` trait, no `RemoteClient` change. ✓

**Placeholder scan:** none — every step has concrete code/commands. Step 5 of Task 4 is explicitly optional bring-up, not a gap.

**Type consistency:** `RemoteChannelInfo { handle, display_name, video_count: Option<u64>, avatar_url: Option<String> }`, `map_channel(&Value, &str) -> Option<RemoteChannelInfo>`, `map_video(&Value, &str, &str) -> RemoteVideo`, `pick_media(&Value) -> Option<String>`, `parse_tokens(&Value) -> Option<OAuthTokens>`, and the three `pub fn` HTTP methods are named/typed identically where defined (Tasks 2/3) and used (Task 4). `RemoteVideo` fields (`id/title/channel/video_url/thumb_url/duration_secs`) match `src/remote.rs`. ✓
```
