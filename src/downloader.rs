//! Running `yt-dlp` in the background and surfacing its progress to the UI.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::thread;

pub enum UrlKind {
    Channel { handle: String },
    Playlist,
    Video,
    Unknown,
}

/// Detects the kind of YouTube URL from a string.
pub fn detect_url_kind(url: &str) -> UrlKind {
    if url.contains("playlist?list=") {
        return UrlKind::Playlist;
    }
    if let Some(h) = extract_after(url, "/@") {
        return UrlKind::Channel { handle: h.to_string() };
    }
    if let Some(h) = extract_after(url, "/channel/") {
        return UrlKind::Channel { handle: h.to_string() };
    }
    if let Some(h) = extract_after(url, "/c/") {
        return UrlKind::Channel { handle: h.to_string() };
    }
    if url.contains("watch?v=") || url.contains("youtu.be/") {
        return UrlKind::Video;
    }
    UrlKind::Unknown
}

fn extract_after<'a>(url: &'a str, marker: &str) -> Option<&'a str> {
    let start = url.find(marker)? + marker.len();
    let rest = &url[start..];
    let end = rest
        .find(|c| c == '/' || c == '?' || c == '&' || c == '#')
        .unwrap_or(rest.len());
    if end == 0 { None } else { Some(&rest[..end]) }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Running,
    Done,
    Failed,
}

enum Msg {
    Line(String),
    Progress(f32),
    Finished(bool),
}

pub struct Job {
    pub url: String,
    pub label: String,
    pub state: JobState,
    pub progress: f32,
    pub log: Vec<String>,
    rx: Receiver<Msg>,
}

impl Job {
    fn drain(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Line(line) => {
                    self.log.push(line);
                    if self.log.len() > 800 {
                        let cut = self.log.len() - 800;
                        self.log.drain(0..cut);
                    }
                }
                Msg::Progress(p) => self.progress = p,
                Msg::Finished(ok) => {
                    self.state = if ok { JobState::Done } else { JobState::Failed };
                }
            }
        }
    }
}

pub struct Downloader {
    pub jobs: Vec<Job>,
    pub channels_root: PathBuf,
}

impl Downloader {
    pub fn new(channels_root: PathBuf) -> Self {
        Self { jobs: Vec::new(), channels_root }
    }

    pub fn start(&mut self, url: String, kind: &UrlKind) {
        let (tx, rx) = channel();
        let archive_path = self.channels_root.join("archive.txt");

        let (out_arg, label) = match kind {
            UrlKind::Channel { handle } => {
                let dir = self.channels_root.join(handle);
                let _ = std::fs::create_dir_all(&dir);
                (
                    format!("{}/%(title)s [%(id)s].%(ext)s", dir.display()),
                    format!("channels/{}/", handle),
                )
            }
            UrlKind::Playlist => (
                format!("{}/%(channel)s/%(playlist_title)s/%(title)s [%(id)s].%(ext)s", self.channels_root.display()),
                "channels/<channel>/<playlist>/".to_string(),
            ),
            UrlKind::Video | UrlKind::Unknown => (
                format!("{}/%(channel)s/%(title)s [%(id)s].%(ext)s", self.channels_root.display()),
                "channels/<channel>/".to_string(),
            ),
        };

        let url_for_thread = url.clone();
        thread::spawn(move || {
            let spawn_result = Command::new("yt-dlp")
                .arg("--newline")
                .arg("--no-color")
                .arg("--no-progress-bar")
                .arg("--write-subs")
                .arg("--write-thumbnail")
                .arg("--write-description")
                .arg("-f")
                .arg("mkv")
                .arg("--embed-metadata")
                .arg("--break-on-existing")
                .arg("--cookies-from-browser")
                .arg("firefox")
                .arg("--download-archive")
                .arg(archive_path.display().to_string())
                .arg("--ignore-errors")
                .arg("-o")
                .arg(&out_arg)
                .arg(&url_for_thread)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn();

            let mut child = match spawn_result {
                Ok(child) => child,
                Err(err) => {
                    let _ = tx.send(Msg::Line(format!("could not launch yt-dlp: {err}")));
                    let _ = tx.send(Msg::Finished(false));
                    return;
                }
            };

            if let Some(stderr) = child.stderr.take() {
                let tx = tx.clone();
                thread::spawn(move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        let _ = tx.send(Msg::Line(format!("[stderr] {line}")));
                    }
                });
            }

            if let Some(stdout) = child.stdout.take() {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if let Some(p) = parse_progress(&line) {
                        let _ = tx.send(Msg::Progress(p));
                    }
                    let _ = tx.send(Msg::Line(line));
                }
            }

            let ok = child.wait().map(|s| s.success()).unwrap_or(false);
            let _ = tx.send(Msg::Finished(ok));
        });

        self.jobs.push(Job {
            url,
            label,
            state: JobState::Running,
            progress: 0.0,
            log: Vec::new(),
            rx,
        });
    }

    pub fn poll(&mut self) {
        for job in &mut self.jobs {
            job.drain();
        }
    }

    pub fn any_running(&self) -> bool {
        self.jobs.iter().any(|j| j.state == JobState::Running)
    }
}

fn parse_progress(line: &str) -> Option<f32> {
    let rest = line.trim_start().strip_prefix("[download]")?.trim_start();
    let pct_end = rest.find('%')?;
    let value: f32 = rest[..pct_end].trim().parse().ok()?;
    Some((value / 100.0).clamp(0.0, 1.0))
}
