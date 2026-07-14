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
const BIN: &str = env!("CARGO_BIN_EXE_catacomb");

/// True if `curl` is usable; the tests no-op otherwise so a machine
/// without curl doesn't show spurious failures.
fn have_curl() -> bool {
    Command::new("curl").arg("--version").stdout(Stdio::null()).stderr(Stdio::null())
        .status().map(|s| s.success()).unwrap_or(false)
}

/// True if `ffmpeg` is usable — the perceptual-dedup test needs it to both
/// generate fixtures and fingerprint them; it skips otherwise.
fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null())
        .status().map(|s| s.success()).unwrap_or(false)
}

/// A running `catacomb --web` child against a scratch dir. Killed and
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
            .expect("spawn catacomb --web");
        let s = Server { child, port, dir };
        s.wait_ready();
        s
    }

    fn wait_ready(&self) {
        // Generous budget: the ffmpeg-heavy dedup test can spike CPU and delay
        // a sibling server's startup, so don't flake under parallel load.
        for _ in 0..400 {
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

    fn put(&self, path: &str, json: &str) -> (u16, String) {
        let mut a = self.req_args(path, "PUT");
        let url = a.pop().unwrap();
        a.extend([
            "-H".into(), "Content-Type: application/json".into(),
            "--data-binary".into(), "@-".into(),
        ]);
        a.push(url);
        curl(&a, Some(json)).expect("curl PUT")
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
    assert!(body.contains("Catacomb"), "index should mention Catacomb");

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
    assert_eq!(field(&body, "sponsorblock_mode"), Some("mark"), "default sponsorblock = mark");

    // Flip a few global settings.
    let (code, _) = s.post("/api/settings", r#"{
        "transcode":true,"scheduler_enabled":false,"scheduler_interval_hours":24,
        "max_concurrent":5,"use_bundled_ytdlp":false,"use_pot_provider":false,
        "subtitles_enabled":true,"subtitles_auto":false,"subtitles_embed":true,
        "subtitle_langs":"en","subtitle_format":"srt","youtube_player_clients":"tv,mweb",
        "sponsorblock_mode":"remove","bind_mode":"all",
        "convert_mode":"h264-mp4","convert_crf":28,"convert_preset":"fast",
        "convert_audio_format":"","convert_keep_original":true
    }"#);
    assert_eq!(code, 200);

    // GET reflects the change…
    let (_, body) = s.get("/api/settings");
    assert_eq!(field(&body, "convert_mode"), Some("h264-mp4"));
    assert_eq!(field(&body, "convert_crf"), Some("28"));
    assert_eq!(field(&body, "youtube_player_clients"), Some("tv,mweb"));
    assert_eq!(field(&body, "sponsorblock_mode"), Some("remove"));
    assert_eq!(field(&body, "subtitle_format"), Some("srt"));
    assert_eq!(
        field(&body, "current_bind").map(|s| s.starts_with("0.0.0.0:")),
        Some(true),
        "bind_mode POST took effect (current_bind now 0.0.0.0): {body}"
    );

    // …and so does config.toml on disk.
    let cfg = std::fs::read_to_string(s.dir.join("config.toml")).unwrap();
    assert!(cfg.contains("mode = \"h264-mp4\""), "config persisted convert mode:\n{cfg}");
    assert!(cfg.contains("youtube_player_clients = \"tv,mweb\""), "config persisted clients");
    assert!(cfg.contains("sponsorblock_mode = \"remove\""), "config persisted sponsorblock");
    assert!(cfg.contains("bind = \"0.0.0.0\""), "config persisted resolved bind address:\n{cfg}");
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
        r#"{"convert_mode":"audio","youtube_player_clients":"tv","sponsorblock_mode":"off","subtitles_enabled":false}"#,
    );
    assert_eq!(code, 200);
    let (_, opts) = s.get("/api/channels/channels/SomeChannel/options");
    assert_eq!(field(&opts, "convert_mode"), Some("audio"));
    assert_eq!(field(&opts, "youtube_player_clients"), Some("tv"));
    assert_eq!(field(&opts, "sponsorblock_mode"), Some("off"));

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

#[test]
fn full_text_search_indexes_titles_and_descriptions() {
    if !have_curl() { eprintln!("skip: no curl"); return; }
    let s = Server::start();

    // Seed a real video on disk: <dir>/ch/channels/TestChan/<Title> [id].mkv
    // plus a .description sidecar (the description word isn't in the title).
    let chan = s.dir.join("ch/channels/TestChan");
    std::fs::create_dir_all(&chan).unwrap();
    std::fs::write(chan.join("Cooking Sourdough [vid123].mkv"), b"").unwrap();
    std::fs::write(chan.join("Cooking Sourdough [vid123].description"),
                   b"an artisan bread tutorial").unwrap();
    // A subtitle sidecar whose spoken words appear nowhere in the title/desc.
    std::fs::write(chan.join("Cooking Sourdough [vid123].en.vtt"),
                   b"WEBVTT\n\n00:00:01.000 --> 00:00:04.000\nwhisk the poolish gently\n").unwrap();

    // Rescan so the server picks up the new file and reindexes it.
    let (code, _) = s.post("/api/rescan", "");
    assert_eq!(code, 200);

    // Title term hits.
    let (code, body) = s.get("/api/search?q=sourdough");
    assert_eq!(code, 200, "{body}");
    assert!(body.contains("vid123"), "title search should hit: {body}");

    // Prefix / type-ahead hits.
    assert!(s.get("/api/search?q=sourd").1.contains("vid123"), "prefix search should hit");

    // A word only in the description hits (proves the sidecar got indexed).
    assert!(s.get("/api/search?q=artisan").1.contains("vid123"), "description search should hit");

    // A word only spoken in the transcript hits (proves the .vtt got indexed).
    assert!(s.get("/api/search?q=poolish").1.contains("vid123"), "transcript search should hit");

    // An unrelated query does not.
    assert!(!s.get("/api/search?q=quantumchromodynamics").1.contains("vid123"),
            "unrelated query must not hit");
}

#[test]
fn podcast_feed_serves_rss_with_enclosures() {
    if !have_curl() { eprintln!("skip: no curl"); return; }
    let s = Server::start();
    let chan = s.dir.join("ch/channels/Demo");
    std::fs::create_dir_all(&chan).unwrap();
    std::fs::write(chan.join("Cool Talk [vidXYZ].mp4"), b"fakevideo").unwrap();
    std::fs::write(chan.join("Cool Talk [vidXYZ].info.json"),
                   br#"{"duration":125.0,"upload_date":"20240102"}"#).unwrap();
    assert_eq!(s.post("/api/rescan", "").0, 200);

    let (code, body) = s.get("/feed.xml");
    assert_eq!(code, 200, "{body}");
    assert!(body.contains("<rss"), "is RSS: {body}");
    assert!(body.contains("<title>Cool Talk</title>"), "item title present: {body}");
    assert!(body.contains("<enclosure"), "has an enclosure: {body}");
    assert!(body.contains("/files/channels/Demo/Cool%20Talk%20%5BvidXYZ%5D.mp4"),
            "enclosure points at the media file: {body}");
    assert!(body.contains(r#"type="video/mp4""#), "correct MIME: {body}");
    assert!(body.contains("Tue, 02 Jan 2024"), "pubDate from upload_date: {body}");
    // The channel feed works too.
    assert_eq!(s.get("/feed/channels/Demo").0, 200);
    // Unknown channel → 404.
    assert_eq!(s.get("/feed/channels/Nope").0, 404);
}

#[test]
fn podcast_feed_token_gates_access_when_password_set() {
    if !have_curl() { eprintln!("skip: no curl"); return; }
    let s = Server::start();
    let chan = s.dir.join("ch/channels/Demo");
    std::fs::create_dir_all(&chan).unwrap();
    std::fs::write(chan.join("Talk [vidT].mp4"), b"x").unwrap();
    assert_eq!(s.post("/api/rescan", "").0, 200);

    // Grab the feed token (UI is unauthenticated until a password is set).
    let (_, info) = s.get("/api/feed-info");
    let token = field(&info, "token").expect("feed token").to_string();
    assert!(!token.is_empty());

    // Set a password — now the UI + API require auth.
    let (code, _) = s.post("/api/settings", &format!(
        r#"{{"transcode":false,"scheduler_enabled":false,"scheduler_interval_hours":24,
            "max_concurrent":3,"use_bundled_ytdlp":false,"use_pot_provider":false,
            "subtitles_enabled":false,"subtitles_auto":false,"subtitles_embed":false,
            "subtitle_langs":"","subtitle_format":"","youtube_player_clients":"",
            "sponsorblock_mode":"mark","convert_mode":"","convert_crf":23,"convert_preset":"",
            "convert_audio_format":"","convert_keep_original":false,
            "new_download_password":"hunter2"}}"#));
    assert_eq!(code, 200);

    // Feed without the token is now rejected…
    assert_eq!(s.get("/feed.xml").0, 401, "feed must be gated once a password is set");
    // …but the tokenized URL still works (a podcast client can't log in).
    assert_eq!(s.get(&format!("/feed.xml?token={token}")).0, 200);
    // A wrong token is rejected.
    assert_eq!(s.get("/feed.xml?token=bogus").0, 401);
    // The media mount is reachable with the token too (so enclosures load).
    assert_eq!(s.get(&format!("/files/channels/Demo/Talk%20%5BvidT%5D.mp4?token={token}")).0, 200);
}

#[test]
fn perceptual_dedup_groups_reencodes() {
    if !have_curl() { eprintln!("skip: no curl"); return; }
    if !have_ffmpeg() { eprintln!("skip: no ffmpeg"); return; }
    let s = Server::start();
    let chan = s.dir.join("ch/channels/Demo");
    std::fs::create_dir_all(&chan).unwrap();

    let gen = |args: &[&str]| {
        let ok = Command::new("ffmpeg").arg("-nostdin").arg("-y").arg("-loglevel").arg("error")
            .args(args).status().map(|st| st.success()).unwrap_or(false);
        assert!(ok, "ffmpeg gen failed: {args:?}");
    };
    let orig = chan.join("orig [aaa].mp4");
    let reenc = chan.join("reenc [bbb].mp4");
    let diff = chan.join("diff [ccc].mp4");
    gen(&["-f","lavfi","-i","testsrc=duration=20:size=640x480:rate=10", orig.to_str().unwrap()]);
    gen(&["-i", orig.to_str().unwrap(), "-vf","scale=320:240","-r","15","-c:v","libx264","-crf","38", reenc.to_str().unwrap()]);
    gen(&["-f","lavfi","-i","testsrc2=duration=20:size=640x480:rate=10", diff.to_str().unwrap()]);
    for stem in ["orig [aaa]", "reenc [bbb]", "diff [ccc]"] {
        std::fs::write(chan.join(format!("{stem}.info.json")), r#"{"duration":20.0}"#).unwrap();
    }

    assert_eq!(s.post("/api/rescan", "").0, 200);
    assert_eq!(s.post("/api/maintenance/dedup/scan", "").0, 200);

    // Poll until the background job finishes (fingerprinting 3 short clips).
    let mut body = String::new();
    for _ in 0..160 {
        let (_, b) = s.get("/api/maintenance/dedup/status");
        if field(&b, "running") == Some("false") { body = b; break; }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    assert!(field(&body, "running") == Some("false"), "dedup job never finished: {body}");
    // orig + its re-encode group together; the unrelated clip stays out.
    assert!(body.contains("aaa"), "group should contain orig: {body}");
    assert!(body.contains("bbb"), "group should contain the re-encode: {body}");
    assert!(!body.contains("ccc"), "unrelated video must not be grouped: {body}");
}

#[test]
fn pwa_assets_served_and_ungated() {
    if !have_curl() { eprintln!("skip: no curl"); return; }
    let s = Server::start();
    s.wait_ready();

    // The SPA advertises the manifest + registers the service worker.
    let (_, idx) = s.get("/");
    assert!(idx.contains("manifest.webmanifest"), "index links the manifest");
    assert!(idx.contains("serviceWorker"), "index registers the service worker");

    let (code, body) = s.get("/manifest.webmanifest");
    assert_eq!(code, 200);
    assert!(body.contains("\"name\": \"Catacomb\""), "manifest body: {body}");
    let (code, body) = s.get("/sw.js");
    assert_eq!(code, 200);
    assert!(body.contains("catacomb-shell"), "sw body: {body}");
    assert_eq!(s.get("/icons/icon-192.png").0, 200);
    assert_eq!(s.get("/icons/icon-512.png").0, 200);
    assert_eq!(s.get("/apple-touch-icon.png").0, 200);
    assert_eq!(s.get("/icons/nope.png").0, 404);

    // Setting a password must NOT gate the PWA statics: the browser fetches
    // the manifest/icons during install without a session, and the login
    // page itself links them.
    let (code, _) = s.post("/api/settings", &format!(
        r#"{{"transcode":false,"scheduler_enabled":false,"scheduler_interval_hours":24,
            "max_concurrent":3,"use_bundled_ytdlp":false,"use_pot_provider":false,
            "subtitles_enabled":false,"subtitles_auto":false,"subtitles_embed":false,
            "subtitle_langs":"","subtitle_format":"","youtube_player_clients":"",
            "sponsorblock_mode":"mark","convert_mode":"","convert_crf":23,"convert_preset":"",
            "convert_audio_format":"","convert_keep_original":false,
            "new_download_password":"hunter2"}}"#));
    assert_eq!(code, 200);
    assert_eq!(s.get("/api/library").0, 401, "API must be gated once a password is set");
    assert_eq!(s.get("/manifest.webmanifest").0, 200, "manifest stays public");
    assert_eq!(s.get("/sw.js").0, 200, "service worker stays public");
    assert_eq!(s.get("/icons/icon-512.png").0, 200, "icons stay public");
}

#[test]
fn remotes_editor_put_get_roundtrip() {
    if !have_curl() { eprintln!("skip: no curl"); return; }
    let s = Server::start();

    // Replace the (empty) peer list with a catacomb + a peertube remote.
    let body = r#"[
        {"name":"peerA","url":"http://a:8081","kind":"catacomb","password":"secret"},
        {"name":"frama","url":"https://framatube.org","kind":"peertube"}
    ]"#;
    let (code, _) = s.put("/api/remotes", body);
    assert_eq!(code, 200, "PUT /api/remotes should succeed");

    let (code, list) = s.get("/api/remotes");
    assert_eq!(code, 200);
    assert!(list.contains("\"kind\":\"catacomb\""), "catacomb kind present: {list}");
    assert!(list.contains("\"kind\":\"peertube\""), "peertube kind present: {list}");
    assert!(list.contains("\"name\":\"peerA\""), "peerA present: {list}");
    assert!(list.contains("\"name\":\"frama\""), "frama present: {list}");
    assert!(list.contains("\"has_password\":true"), "peerA has_password true: {list}");
    // Passwords are write-only: the plaintext must never be echoed back.
    assert!(!list.contains("secret"), "GET must not leak the password: {list}");

    // It persisted to config.toml too.
    let cfg = std::fs::read_to_string(s.dir.join("config.toml")).unwrap();
    assert!(cfg.contains("framatube.org"), "peertube remote saved to config: {cfg}");
    assert!(cfg.contains("peertube"), "kind saved to config: {cfg}");

    // Removing one entry via PUT drops it from GET.
    let (code, _) = s.put("/api/remotes", r#"[{"name":"peerA","url":"http://a:8081","kind":"catacomb"}]"#);
    assert_eq!(code, 200);
    let (_, list) = s.get("/api/remotes");
    assert!(!list.contains("frama"), "removed peer gone from GET: {list}");
    // The blank-password edit of peerA kept its stored secret.
    assert!(list.contains("\"has_password\":true"), "peerA password preserved on blank edit: {list}");
}

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
