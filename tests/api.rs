//! End-to-end integration tests for the web API.
//!
//! These spawn the **real** compiled binary in `--web` mode against a
//! throwaway library dir and drive its HTTP endpoints, exercising the
//! full axum + SQLite + config-persistence stack the way a browser would.
//! No network / yt-dlp is needed — every endpoint here is local state.
//!
//! HTTP is done via `curl` (transparently handles the server's gzip +
//! chunked encoding, and is already a runtime/CI dependency). If curl
//! isn't on PATH the suite skips rather than fails.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};

/// Absolute path to the binary under test (set by Cargo for integration
/// tests of a crate that builds a binary).
const BIN: &str = env!("CARGO_BIN_EXE_yt-offline");

/// True if `curl` is usable; the tests no-op otherwise so a machine
/// without curl doesn't show spurious failures.
fn have_curl() -> bool {
    Command::new("curl").arg("--version").stdout(Stdio::null()).stderr(Stdio::null())
        .status().map(|s| s.success()).unwrap_or(false)
}

/// A running `yt-offline --web` child against a scratch dir. Killed and
/// its dir removed on drop.
struct Server {
    child: Child,
    port: u16,
    dir: PathBuf,
}

impl Server {
    fn start() -> Server {
        static N: AtomicU16 = AtomicU16::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("ytoff-it-{}-{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.toml"),
            format!("[backup]\ndirectory = \"{}/ch\"\n", dir.display()),
        ).unwrap();

        let port = free_port();
        let child = Command::new(BIN)
            .arg("--web").arg(port.to_string())
            .current_dir(&dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn yt-offline --web");
        let s = Server { child, port, dir };
        s.wait_ready();
        s
    }

    fn wait_ready(&self) {
        for _ in 0..150 {
            if let Some((code, _)) = curl(&self.req_args("/", "GET"), None) {
                if code != 0 { return; } // 0 = connection refused (not up yet)
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("server never became ready on port {}", self.port);
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }

    /// Base curl args: silent, append `\n__S__<status>` after the body so
    /// the status code is recoverable without a separate request.
    fn req_args(&self, path: &str, method: &str) -> Vec<String> {
        let mut a: Vec<String> = vec![
            "-s".into(), "-o".into(), "-".into(),
            "-w".into(), "\n__S__%{http_code}".into(),
            "--max-time".into(), "15".into(),
            "-X".into(), method.into(),
        ];
        a.push(self.url(path));
        a
    }

    fn get(&self, path: &str) -> (u16, String) {
        curl(&self.req_args(path, "GET"), None).expect("curl GET")
    }

    fn post(&self, path: &str, json: &str) -> (u16, String) {
        let mut a = self.req_args(path, "POST");
        // Insert content-type + stdin body before the URL (last element).
        let url = a.pop().unwrap();
        a.extend([
            "-H".into(), "Content-Type: application/json".into(),
            "--data-binary".into(), "@-".into(),
        ]);
        a.push(url);
        curl(&a, Some(json)).expect("curl POST")
    }

    fn delete(&self, path: &str) -> (u16, String) {
        curl(&self.req_args(path, "DELETE"), None).expect("curl DELETE")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Run curl with `args`, optionally piping `stdin` to it. Returns
/// `(http_status, body)`, or `None` if curl couldn't run. Status 0 means
/// the connection failed (curl's `000`).
fn curl(args: &[String], stdin: Option<&str>) -> Option<(u16, String)> {
    let mut c = Command::new("curl");
    c.args(args).stdout(Stdio::piped()).stderr(Stdio::null());
    c.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    let mut child = c.spawn().ok()?;
    if let Some(b) = stdin {
        child.stdin.take()?.write_all(b.as_bytes()).ok()?;
    }
    let out = child.wait_with_output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let (body, code) = s.rsplit_once("\n__S__")?;
    Some((code.trim().parse().ok()?, body.to_string()))
}

/// Pull a top-level JSON string/number/bool field out of a flat object
/// without a JSON dep — good enough for asserting a single value. Handles
/// quoted string values that contain commas (e.g. "tv,mweb").
fn field<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\":");
    let i = body.find(&needle)? + needle.len();
    let rest = body[i..].trim_start();
    if let Some(after_quote) = rest.strip_prefix('"') {
        // Quoted string: read to the closing quote (no escape handling
        // needed for the simple values these tests assert on).
        let end = after_quote.find('"')?;
        Some(&after_quote[..end])
    } else {
        // Bare number / bool / null: read to the next , or }.
        let end = rest.find([',', '}']).unwrap_or(rest.len());
        Some(rest[..end].trim())
    }
}

// ── tests ────────────────────────────────────────────────────────────────

#[test]
fn index_and_library_served() {
    if !have_curl() { eprintln!("skip: no curl"); return; }
    let s = Server::start();

    let (code, body) = s.get("/");
    assert_eq!(code, 200, "GET / should serve the SPA");
    assert!(body.contains("yt-offline"), "index should mention yt-offline");

    let (code, body) = s.get("/api/library");
    assert_eq!(code, 200);
    assert!(body.contains("\"channels\""), "library payload has channels: {body}");
}

#[test]
fn library_etag_returns_304() {
    if !have_curl() { eprintln!("skip: no curl"); return; }
    let s = Server::start();
    // Grab the ETag via a header dump, then re-request with If-None-Match.
    let etag = {
        let args: Vec<String> = vec![
            "-s".into(), "-D".into(), "-".into(), "-o".into(), "/dev/null".into(),
            "-w".into(), "\n__S__%{http_code}".into(), "--max-time".into(), "15".into(),
            s.url("/api/library"),
        ];
        let (_, headers) = curl(&args, None).expect("curl headers");
        headers.lines()
            .find_map(|l| l.to_ascii_lowercase().starts_with("etag:")
                .then(|| l.splitn(2, ':').nth(1).unwrap().trim().to_string()))
            .expect("ETag header present")
    };
    let mut a = s.req_args("/api/library", "GET");
    let url = a.pop().unwrap();
    a.extend(["-H".into(), format!("If-None-Match: {etag}")]);
    a.push(url);
    let (code, _) = curl(&a, None).expect("conditional GET");
    assert_eq!(code, 304, "matching ETag should 304");
}

#[test]
fn settings_roundtrip_and_persist() {
    if !have_curl() { eprintln!("skip: no curl"); return; }
    let s = Server::start();

    let (code, body) = s.get("/api/settings");
    assert_eq!(code, 200);
    assert_eq!(field(&body, "convert_mode"), Some(""), "default convert off");

    // Flip a few global settings.
    let (code, _) = s.post("/api/settings", r#"{
        "transcode":true,"scheduler_enabled":false,"scheduler_interval_hours":24,
        "max_concurrent":5,"use_bundled_ytdlp":false,"use_pot_provider":false,
        "subtitles_enabled":true,"subtitles_auto":false,"subtitles_embed":true,
        "subtitle_langs":"en","subtitle_format":"srt","youtube_player_clients":"tv,mweb",
        "convert_mode":"h264-mp4","convert_crf":28,"convert_preset":"fast",
        "convert_audio_format":"","convert_keep_original":true
    }"#);
    assert_eq!(code, 200);

    // GET reflects the change…
    let (_, body) = s.get("/api/settings");
    assert_eq!(field(&body, "convert_mode"), Some("h264-mp4"));
    assert_eq!(field(&body, "convert_crf"), Some("28"));
    assert_eq!(field(&body, "youtube_player_clients"), Some("tv,mweb"));
    assert_eq!(field(&body, "subtitle_format"), Some("srt"));

    // …and so does config.toml on disk.
    let cfg = std::fs::read_to_string(s.dir.join("config.toml")).unwrap();
    assert!(cfg.contains("mode = \"h264-mp4\""), "config persisted convert mode:\n{cfg}");
    assert!(cfg.contains("youtube_player_clients = \"tv,mweb\""), "config persisted clients");
}

#[test]
fn folders_crud_and_cycle_guard() {
    if !have_curl() { eprintln!("skip: no curl"); return; }
    let s = Server::start();

    let (code, body) = s.post("/api/folders", r#"{"name":"Music"}"#);
    assert_eq!(code, 200, "create folder: {body}");
    let id = field(&body, "id").expect("new folder id").to_string();

    // Library payload now lists it.
    let (_, lib) = s.get("/api/library");
    assert!(lib.contains("\"Music\""), "folder appears in library");

    // Rename.
    let (code, _) = s.post(&format!("/api/folders/{id}/rename"), r#"{"name":"Tunes"}"#);
    assert_eq!(code, 200);
    let (_, lib) = s.get("/api/library");
    assert!(lib.contains("\"Tunes\"") && !lib.contains("\"Music\""), "rename took effect");

    // A folder can't be its own parent — cycle guard returns 400.
    let (code, _) = s.post(&format!("/api/folders/{id}/parent"), &format!("{{\"parent_id\":{id}}}"));
    assert_eq!(code, 400, "self-parent is rejected");

    // Delete.
    let (code, _) = s.delete(&format!("/api/folders/{id}"));
    assert_eq!(code, 200);
    let (_, lib) = s.get("/api/library");
    assert!(!lib.contains("\"Tunes\""), "folder gone after delete");
}

#[test]
fn notes_roundtrip() {
    if !have_curl() { eprintln!("skip: no curl"); return; }
    let s = Server::start();

    let (code, _) = s.post("/api/notes/video/abc123", r#"{"body":"remember this clip"}"#);
    assert_eq!(code, 200);
    let (_, notes) = s.get("/api/notes");
    assert!(notes.contains("remember this clip"), "note appears: {notes}");
    assert!(notes.contains("video:abc123"), "note keyed by kind:id");

    // Empty body deletes it.
    let (code, _) = s.post("/api/notes/video/abc123", r#"{"body":"  "}"#);
    assert_eq!(code, 200);
    let (_, notes) = s.get("/api/notes");
    assert!(!notes.contains("remember this clip"), "empty note removed");
}

#[test]
fn channel_options_roundtrip_and_clear() {
    if !have_curl() { eprintln!("skip: no curl"); return; }
    let s = Server::start();

    // Store an override.
    let (code, _) = s.post(
        "/api/channels/channels/SomeChannel/options",
        r#"{"convert_mode":"audio","youtube_player_clients":"tv","subtitles_enabled":false}"#,
    );
    assert_eq!(code, 200);
    let (_, opts) = s.get("/api/channels/channels/SomeChannel/options");
    assert_eq!(field(&opts, "convert_mode"), Some("audio"));
    assert_eq!(field(&opts, "youtube_player_clients"), Some("tv"));

    // An all-default body hits the is_empty() delete path → back to defaults.
    let (code, _) = s.post("/api/channels/channels/SomeChannel/options", r#"{}"#);
    assert_eq!(code, 200);
    let (_, opts) = s.get("/api/channels/channels/SomeChannel/options");
    // convert_mode is Option<String> → serializes null when cleared.
    assert_eq!(field(&opts, "convert_mode"), Some("null"));
}

#[test]
fn backup_db_returns_sqlite() {
    if !have_curl() { eprintln!("skip: no curl"); return; }
    let s = Server::start();
    // Touch the DB so it exists on disk (creating a note writes to it).
    let _ = s.post("/api/notes/video/x", r#"{"body":"y"}"#);
    let (code, body) = s.get("/api/backup/db");
    assert_eq!(code, 200);
    assert!(body.starts_with("SQLite format 3"), "backup is a SQLite file");
}
