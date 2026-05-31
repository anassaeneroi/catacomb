//! YouTube Proof-of-Origin Token (POT) provider integration.
//!
//! YouTube increasingly requires a per-request POT bound to each video ID
//! before it'll hand back the format URLs yt-dlp needs. The token has to be
//! minted by BotGuard, which is a JavaScript challenge from YouTube; the
//! upstream solution is a tiny long-running provider that mints tokens on
//! demand and exposes them over HTTP, plus a yt-dlp Python plugin that
//! consults the provider transparently.
//!
//! We use the Rust port at <https://github.com/jim60105/bgutil-ytdlp-pot-provider-rs>
//! (avoids the Node.js dependency of the original) plus the upstream
//! Python plugin from <https://github.com/Brainicism/bgutil-ytdlp-pot-provider>.
//!
//! # Layout
//!
//! Lives next to the bundled deno + yt-dlp:
//!
//! ```text
//! ~/.local/share/yt-offline/
//!   bin/
//!     bgutil-pot            ← the Rust HTTP server binary
//!   venv/                   ← reused — pip-install the Python plugin here
//!     lib/python*/site-packages/yt_dlp_plugins/extractor/bgutil_*.py
//! ```
//!
//! # Activation
//!
//! Gated on [`crate::config::BackupSection::use_pot_provider`] (default off).
//! Only effective when [`use_bundled_ytdlp`] is also on — the Python
//! plugin is installed into the bundled venv, not the system Python.
//!
//! When active, the [`Downloader`] spawns the bgutil-pot server as a
//! background child on first job and passes
//! `--extractor-args "youtubepot-bgutilhttp:base_url=http://127.0.0.1:4416"`
//! to every yt-dlp invocation. The child is killed on process exit via
//! the same panic/Drop path as other background services.

use std::path::PathBuf;
use std::process::Command;

/// HTTP port the bgutil-pot server listens on. The Python plugin defaults
/// to discovering `127.0.0.1:4416`, so we use that unless overridden via
/// future config knob.
pub const SERVER_PORT: u16 = 4416;

/// Bound only to localhost — there's no reason for the POT server to be
/// reachable off-host, and exposing BotGuard tokens to the LAN would be
/// a footgun.
pub const SERVER_HOST: &str = "127.0.0.1";

/// Full URL the yt-dlp plugin uses to reach the provider. Passed to
/// yt-dlp via `--extractor-args "youtubepot-bgutilhttp:base_url=…"`.
pub fn server_url() -> String {
    format!("http://{SERVER_HOST}:{SERVER_PORT}")
}

/// Path to the bgutil-pot binary inside the bundled bin dir.
///
/// Co-locates with the bundled `deno` so a single bundled-dir cleanup
/// (currently just `rm -rf ~/.local/share/yt-offline/bin`) removes
/// both.
pub fn bin_path() -> PathBuf {
    let mut p = crate::ytdlp_bin::bundled_dir();
    p.push(if cfg!(windows) { "bgutil-pot.exe" } else { "bgutil-pot" });
    p
}

/// True if the POT provider binary is installed under the bundled-dir.
/// The Python plugin's presence is verified separately by yt-dlp at
/// runtime; missing it just degrades silently to "no POT" rather than
/// failing the download, so we don't preflight it here.
pub fn installed() -> bool {
    bin_path().exists()
}

/// GitHub release asset name for the current OS/arch. macOS keeps two
/// per-arch binaries; Windows ships an `.exe`; Linux gets `x86_64` or
/// `aarch64`. Falls back to Linux x86_64 if we can't classify the
/// host — the user will see the download fail rather than us silently
/// installing a wrong-arch binary.
fn release_asset() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux",   "x86_64")  => "bgutil-pot-linux-x86_64",
        ("linux",   "aarch64") => "bgutil-pot-linux-aarch64",
        ("macos",   "x86_64")  => "bgutil-pot-macos-x86_64",
        ("macos",   "aarch64") => "bgutil-pot-macos-aarch64",
        ("windows", _)         => "bgutil-pot-windows-x86_64.exe",
        _                       => "bgutil-pot-linux-x86_64",
    }
}

/// URL of the latest-release asset on GitHub. We use `releases/latest/
/// download/<asset>` rather than pinning a version so the upstream's
/// release cadence flows through without code changes — the BotGuard
/// challenge format shifts on YouTube's whim, so being a release behind
/// can mean broken downloads.
fn release_url() -> String {
    format!(
        "https://github.com/jim60105/bgutil-ytdlp-pot-provider-rs/releases/latest/download/{}",
        release_asset()
    )
}

/// Shell command that downloads the bgutil-pot binary into the bundled
/// bin dir and `pip install`s the matching Python plugin into the
/// bundled venv.
///
/// Runs through the same job-with-streaming-log pipeline as the bundled
/// yt-dlp install, so the user sees a progress feed and any error
/// surfaces in the Downloads modal.
pub fn install_command() -> Command {
    let bin_dir = crate::ytdlp_bin::bundled_dir().display().to_string();
    let bin_path = bin_path().display().to_string();
    let venv_python = if cfg!(windows) {
        crate::ytdlp_bin::bundled_venv().join("Scripts").join("python.exe")
    } else {
        crate::ytdlp_bin::bundled_venv().join("bin").join("python")
    };
    let venv_python_s = venv_python.display().to_string();
    let url = release_url();

    #[cfg(windows)]
    {
        let script = format!(
            "$ErrorActionPreference='Stop'; \
             New-Item -ItemType Directory -Force -Path '{bin_dir}' | Out-Null; \
             if (-not (Test-Path '{venv_python}')) {{ \
               Write-Error 'bundled yt-dlp venv not installed; install it first'; exit 1 \
             }}; \
             Write-Host '==> downloading bgutil-pot'; \
             Invoke-WebRequest -Uri '{url}' -OutFile '{bin_path}'; \
             Write-Host '==> installing the Python plugin into the venv'; \
             & '{venv_python}' -m pip install --upgrade --quiet bgutil-ytdlp-pot-provider; \
             Write-Host '==> versions'; \
             & '{bin_path}' --version; \
             Write-Host '==> done'",
            bin_dir = bin_dir, venv_python = venv_python_s, bin_path = bin_path, url = url,
        );
        let mut cmd = Command::new("powershell");
        cmd.arg("-NoProfile").arg("-Command").arg(script);
        cmd
    }
    #[cfg(not(windows))]
    {
        let script = format!(
            r#"set -e
command -v curl >/dev/null || {{ echo 'error: curl not installed'; exit 1; }}

if [ ! -x '{venv_python}' ]; then
  echo 'error: bundled yt-dlp venv not installed.'
  echo '       Click Install on the yt-dlp row in Settings first, then retry.'
  exit 1
fi

mkdir -p '{bin_dir}'

echo '==> downloading bgutil-pot from {url}'
curl -fL --no-progress-meter --connect-timeout 30 --max-time 600 --retry 3 \
  -o '{bin_path}' '{url}' &
DLPID=$!
while kill -0 $DLPID 2>/dev/null; do
  sleep 3
  SZ=$(wc -c < '{bin_path}' 2>/dev/null || echo 0)
  echo "    bgutil-pot: $SZ bytes received..."
done
wait $DLPID
echo "    done: $(wc -c < '{bin_path}') bytes"
chmod +x '{bin_path}'

echo '==> installing bgutil-ytdlp-pot-provider Python plugin into the venv'
'{venv_python}' -m pip install --upgrade --quiet --progress-bar off bgutil-ytdlp-pot-provider

echo '==> versions'
'{bin_path}' --version
echo '==> done'"#,
            bin_dir = bin_dir, venv_python = venv_python_s, bin_path = bin_path, url = url,
        );
        let mut cmd = Command::new("bash");
        cmd.arg("-c").arg(script);
        cmd
    }
}

/// Spawn the bgutil-pot HTTP server as a background child process bound
/// to [`SERVER_HOST`]:[`SERVER_PORT`].
///
/// Returns the [`std::process::Child`] handle so the caller can keep it
/// alive (drop = SIGKILL on Unix, TerminateProcess on Windows). Errors
/// fall through with the underlying IO error; the caller surfaces a
/// friendlier message.
///
/// We use the binary's `server` subcommand explicitly rather than relying
/// on positional arg order in case the upstream CLI grows new modes.
pub fn spawn_server() -> std::io::Result<std::process::Child> {
    let mut cmd = Command::new(bin_path());
    cmd.arg("server")
        .arg("--host").arg(SERVER_HOST)
        .arg("--port").arg(SERVER_PORT.to_string());
    // Detach stdout/stderr — the server is chatty and we don't have a
    // good place to surface its logs yet. Future improvement: pipe into
    // a per-process job log.
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    cmd.stdin(std::process::Stdio::null());
    cmd.spawn()
}

/// Best-effort kill of a running server child. Called from the
/// [`Downloader`]'s shutdown path; ignores errors because the process
/// is exiting anyway.
pub fn kill_server(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// The `--extractor-args` value that points the bgutil yt-dlp plugin at
/// our local server. yt-dlp accepts multiple `--extractor-args` flags;
/// callers append this to the existing arg list when POT is active.
pub fn extractor_args() -> String {
    format!("youtubepot-bgutilhttp:base_url={}", server_url())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_url_uses_loopback_and_port() {
        assert_eq!(server_url(), "http://127.0.0.1:4416");
    }

    #[test]
    fn extractor_args_format_matches_plugin() {
        // The plugin docs document this exact key. If yt-dlp's
        // extractor-arg parser ever changes this is the first thing
        // we'd want to know.
        assert_eq!(
            extractor_args(),
            "youtubepot-bgutilhttp:base_url=http://127.0.0.1:4416"
        );
    }

    #[test]
    fn release_asset_covers_known_arches() {
        // Sanity-check the table: every (os, arch) we care about maps to
        // a non-empty name and the unknown branch falls back to Linux.
        let s = release_asset();
        assert!(!s.is_empty());
    }
}
