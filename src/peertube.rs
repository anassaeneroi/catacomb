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
#[allow(dead_code)] // consumed by the browse UI in phase 3
pub struct RemoteChannelInfo {
    pub handle: String,
    pub display_name: String,
    pub video_count: Option<u64>,
    pub avatar_url: Option<String>,
}

#[allow(dead_code)] // constructed + driven by the editor (phase 2) and browse UI (phase 3)
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

#[allow(dead_code)] // public API consumed by phases 2-3 (editor + browse UI)
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

    /// Fetch the instance's local OAuth client id/secret (needed for every
    /// token grant).
    fn oauth_client(&self) -> Result<(String, String), String> {
        let oc: Value = self
            .client
            .get(format!("{}/api/v1/oauth-clients/local", self.api_base))
            .send()
            .map_err(|e| format!("peertube {}: oauth-clients: {e}", self.name))?
            .json()
            .map_err(|e| format!("peertube {}: oauth-clients parse: {e}", self.name))?;
        let id = oc.get("client_id").and_then(Value::as_str).unwrap_or("").to_string();
        let secret = oc.get("client_secret").and_then(Value::as_str).unwrap_or("").to_string();
        Ok((id, secret))
    }

    /// POST a token grant (`form` fields already include client id/secret and
    /// grant-specific params), cache the result, and return the access token.
    fn token_grant(&self, form: &[(&str, &str)]) -> Result<String, String> {
        let resp = self
            .client
            .post(format!("{}/api/v1/users/token", self.api_base))
            .form(form)
            .send()
            .map_err(|e| format!("peertube {}: token: {e}", self.name))?;
        if !resp.status().is_success() {
            return Err(format!("peertube {}: token HTTP {}", self.name, resp.status().as_u16()));
        }
        let v: Value = resp
            .json()
            .map_err(|e| format!("peertube {}: token parse: {e}", self.name))?;
        let toks = parse_tokens(&v).ok_or_else(|| format!("peertube {}: token missing", self.name))?;
        let access = toks.access.clone();
        *self.tokens.lock().unwrap() = Some(toks);
        Ok(access)
    }

    /// Obtain (and cache) an access token via the OAuth2 password grant. No-op /
    /// returns None when no credentials are configured (public browsing).
    fn ensure_token(&self) -> Result<Option<String>, String> {
        let (Some(user), Some(pass)) = (&self.username, &self.password) else {
            return Ok(None);
        };
        if let Some(t) = self.tokens.lock().unwrap().as_ref() {
            return Ok(Some(t.access.clone()));
        }
        let (id, secret) = self.oauth_client()?;
        let access = self.token_grant(&[
            ("client_id", &id),
            ("client_secret", &secret),
            ("grant_type", "password"),
            ("username", user.as_str()),
            ("password", pass.as_str()),
        ])?;
        Ok(Some(access))
    }

    /// Exchange the cached refresh token for a fresh access token. Returns
    /// `Ok(None)` when there's no usable refresh token or the grant is rejected,
    /// so the caller can fall back to a full password re-grant.
    fn try_refresh(&self) -> Result<Option<String>, String> {
        let refresh = match self.tokens.lock().unwrap().as_ref() {
            Some(t) if !t.refresh.is_empty() => t.refresh.clone(),
            _ => return Ok(None),
        };
        let (id, secret) = self.oauth_client()?;
        match self.token_grant(&[
            ("client_id", &id),
            ("client_secret", &secret),
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
        ]) {
            Ok(access) => Ok(Some(access)),
            Err(_) => Ok(None),
        }
    }

    /// GET an API path, adding a Bearer token when credentials are set. On a
    /// 401, refresh via the refresh token, falling back to a full re-grant.
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
            // Try refresh first; on failure clear the cache and re-grant.
            let next = match self.try_refresh()? {
                Some(access) => Some(access),
                None => {
                    *self.tokens.lock().unwrap() = None;
                    self.ensure_token()?
                }
            };
            send(&next).map_err(|e| format!("peertube {}: {path}: {e}", self.name))?
        } else {
            resp
        };
        if !resp.status().is_success() {
            return Err(format!("peertube {}: {path}: HTTP {}", self.name, resp.status().as_u16()));
        }
        resp.json().map_err(|e| format!("peertube {}: {path}: parse {e}", self.name))
    }

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
}

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
    let res = |f: &Value| {
        f.get("resolution")
            .and_then(|r| r.get("id"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
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
    let refresh = v
        .get("refresh_token")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Some(OAuthTokens { access, refresh })
}

#[cfg(test)]
mod tests {
    use super::*;
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
