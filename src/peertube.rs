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
