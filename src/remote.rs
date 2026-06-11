//! Federation — read-only browsing of a *peer* yt-offline instance's library.
//!
//! A [`RemoteClient`] talks to another instance over its existing web API:
//! it fetches `/api/library`, logging in first if the peer has a password
//! (the blocking reqwest client keeps the session cookie). Media is **not**
//! proxied — instead the library JSON's `/files/…` URLs are rewritten to
//! absolute peer URLs with the peer's read-only **feed token** appended, which
//! the peer's auth middleware already accepts for `GET /files/` without a
//! login. So the browser (or mpv) streams video straight from the peer while
//! only the small library JSON travels through us.
//!
//! The client is blocking; the async web layer calls it via
//! `spawn_blocking`, the desktop app calls it directly. Roadmap 3.5.

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::config::RemoteSection;

/// A connection to one peer instance. Cheap to hold; the underlying reqwest
/// client owns a connection pool + cookie jar reused across calls.
pub struct RemoteClient {
    pub name: String,
    /// Base URL with any trailing slash trimmed.
    base: String,
    password: Option<String>,
    client: reqwest::blocking::Client,
    /// Cached read-only feed token (fetched once via `/api/feed-info`).
    feed_token: Mutex<Option<String>>,
}

/// Reduced per-video view for the desktop remote browser (the web UI consumes
/// the full proxied JSON directly). URLs are already absolute + tokenized.
#[derive(Clone, Serialize)]
pub struct RemoteVideo {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub video_url: Option<String>,
    pub thumb_url: Option<String>,
    pub duration_secs: Option<f64>,
}

#[derive(Clone, Serialize)]
pub struct RemoteChannel {
    pub name: String,
    pub videos: Vec<RemoteVideo>,
}

/// A peer's library flattened to channels → videos for read-only display.
#[derive(Clone, Serialize)]
pub struct RemoteLibrary {
    pub channels: Vec<RemoteChannel>,
}

impl RemoteClient {
    pub fn new(cfg: &RemoteSection) -> Self {
        let client = reqwest::blocking::Client::builder()
            .cookie_store(true)
            .timeout(Duration::from_secs(30))
            .user_agent("yt-offline-federation")
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        RemoteClient {
            name: cfg.name.clone(),
            base: cfg.url.trim_end_matches('/').to_string(),
            password: cfg.password.clone().filter(|p| !p.is_empty()),
            client,
            feed_token: Mutex::new(None),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    /// POST the peer's `/api/login` with the configured password. No-op for an
    /// open peer. On success the session cookie lands in the client's jar.
    fn login(&self) -> Result<(), String> {
        let Some(pw) = &self.password else { return Ok(()) };
        let resp = self
            .client
            .post(format!("{}/api/login", self.base))
            .json(&serde_json::json!({ "password": pw }))
            .send()
            .map_err(|e| format!("login request failed: {e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("login rejected: HTTP {}", resp.status().as_u16()))
        }
    }

    /// GET a peer path, logging in + retrying once on a 401 when a password is
    /// configured (covers both first contact and an expired session).
    fn authed_get(&self, path: &str) -> Result<reqwest::blocking::Response, String> {
        let url = format!("{}{}", self.base, path);
        let resp = self
            .client
            .get(&url)
            .send()
            .map_err(|e| format!("request to {} failed: {e}", self.name))?;
        if resp.status().as_u16() == 401 && self.password.is_some() {
            self.login()?;
            return self
                .client
                .get(&url)
                .send()
                .map_err(|e| format!("request to {} failed: {e}", self.name));
        }
        Ok(resp)
    }

    /// Fetch + cache the peer's read-only feed token, used to tokenize media URLs.
    fn feed_token(&self) -> Result<String, String> {
        if let Some(t) = self.feed_token.lock().unwrap().clone() {
            return Ok(t);
        }
        let resp = self.authed_get("/api/feed-info")?;
        if !resp.status().is_success() {
            return Err(format!("feed-info: HTTP {}", resp.status().as_u16()));
        }
        let v: Value = resp.json().map_err(|e| format!("feed-info parse: {e}"))?;
        let token = v.get("token").and_then(Value::as_str).unwrap_or("").to_string();
        *self.feed_token.lock().unwrap() = Some(token.clone());
        Ok(token)
    }

    /// Fetch the peer's `/api/library`, with every `/files/` + `/music-files/`
    /// URL rewritten to an absolute, token-bearing peer URL the browser/mpv can
    /// load directly. Returns the full JSON so the web UI can reuse its grid.
    pub fn library_json(&self) -> Result<Value, String> {
        let resp = self.authed_get("/api/library")?;
        if !resp.status().is_success() {
            return Err(format!("library: HTTP {}", resp.status().as_u16()));
        }
        let mut v: Value = resp.json().map_err(|e| format!("library parse: {e}"))?;
        // Best-effort: if the token can't be fetched, media just won't load,
        // but the listing is still useful.
        let token = self.feed_token().unwrap_or_default();
        rewrite_media_urls(&mut v, &self.base, &token);
        Ok(v)
    }

    /// Parse the (already-rewritten) library JSON into the reduced shape the
    /// desktop browser renders.
    pub fn library(&self) -> Result<RemoteLibrary, String> {
        let v = self.library_json()?;
        Ok(parse_library(&v))
    }
}

/// Whether a relative URL points at peer media the read-only feed token grants
/// (raw files, music, transcoded streams, and subtitle tracks) — i.e. the paths
/// the peer's auth middleware lets a token'd GET through. Mirror this list with
/// the one in `web::auth_middleware`.
fn is_peer_media_path(s: &str) -> bool {
    s.starts_with("/files/")
        || s.starts_with("/music-files/")
        || s.starts_with("/api/transcode/")
        || s.starts_with("/api/sub-vtt/")
}

/// Recursively rewrite media URLs in a library JSON value: any string under a
/// key named `url` or ending in `_url` that points at peer media (see
/// [`is_peer_media_path`]) becomes `{base}{path}?token={token}` so it resolves
/// on the peer without a login. Other URLs (external channel links) are left
/// untouched.
fn rewrite_media_urls(v: &mut Value, base: &str, token: &str) {
    match v {
        Value::Object(map) => {
            for (k, val) in map.iter_mut() {
                if let Value::String(s) = val {
                    let is_url_key = k == "url" || k.ends_with("_url");
                    if is_url_key && is_peer_media_path(s) {
                        let sep = if s.contains('?') { '&' } else { '?' };
                        *s = format!("{base}{s}{sep}token={token}");
                    }
                } else {
                    rewrite_media_urls(val, base, token);
                }
            }
        }
        Value::Array(arr) => {
            for val in arr.iter_mut() {
                rewrite_media_urls(val, base, token);
            }
        }
        _ => {}
    }
}

/// Flatten the web library JSON into channels → videos (incl. playlist videos).
fn parse_library(v: &Value) -> RemoteLibrary {
    let mut channels = Vec::new();
    let Some(chs) = v.get("channels").and_then(Value::as_array) else {
        return RemoteLibrary { channels };
    };
    for ch in chs {
        let name = ch.get("name").and_then(Value::as_str).unwrap_or("?").to_string();
        let mut videos = Vec::new();
        // Top-level videos plus any nested playlist videos.
        let mut collect = |arr: Option<&Vec<Value>>| {
            if let Some(arr) = arr {
                for vid in arr {
                    videos.push(RemoteVideo {
                        id: vid.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
                        title: vid.get("title").and_then(Value::as_str).unwrap_or("(untitled)").to_string(),
                        channel: name.clone(),
                        video_url: vid.get("video_url").and_then(Value::as_str).map(String::from),
                        thumb_url: vid.get("thumb_url").and_then(Value::as_str).map(String::from),
                        duration_secs: vid.get("duration_secs").and_then(Value::as_f64),
                    });
                }
            }
        };
        collect(ch.get("videos").and_then(Value::as_array));
        if let Some(playlists) = ch.get("playlists").and_then(Value::as_array) {
            for pl in playlists {
                collect(pl.get("videos").and_then(Value::as_array));
            }
        }
        channels.push(RemoteChannel { name, videos });
    }
    RemoteLibrary { channels }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rewrites_only_media_urls() {
        let mut v = json!({
            "channels": [{
                "name": "Chan",
                "thumb_url": "/files/channels/chan/folder.jpg",
                "channel_url": "https://youtube.com/@chan",
                "videos": [{
                    "id": "abc",
                    "title": "Vid",
                    "video_url": "/api/transcode/abc",
                    "thumb_url": "/files/channels/chan/abc.webp",
                    "subtitles": [{"url": "/api/sub-vtt/x.vtt"}]
                }]
            }]
        });
        rewrite_media_urls(&mut v, "http://peer:8081", "TOK");
        let vid = &v["channels"][0]["videos"][0];
        // Transcode streams, raw files, and subtitle tracks are all tokenized.
        assert_eq!(vid["video_url"], "http://peer:8081/api/transcode/abc?token=TOK");
        assert_eq!(vid["thumb_url"], "http://peer:8081/files/channels/chan/abc.webp?token=TOK");
        assert_eq!(vid["subtitles"][0]["url"], "http://peer:8081/api/sub-vtt/x.vtt?token=TOK");
        assert_eq!(v["channels"][0]["thumb_url"], "http://peer:8081/files/channels/chan/folder.jpg?token=TOK");
        // External channel links are left alone.
        assert_eq!(v["channels"][0]["channel_url"], "https://youtube.com/@chan");
    }

    #[test]
    fn parses_channels_and_playlist_videos() {
        let v = json!({
            "channels": [{
                "name": "Chan",
                "videos": [{"id": "a", "title": "A", "video_url": "u1"}],
                "playlists": [{"videos": [{"id": "b", "title": "B", "video_url": "u2"}]}]
            }]
        });
        let lib = parse_library(&v);
        assert_eq!(lib.channels.len(), 1);
        assert_eq!(lib.channels[0].videos.len(), 2);
        assert_eq!(lib.channels[0].videos[1].id, "b");
    }
}
