//! Management of bundled yt-dlp + deno binaries.
//!
//! When [`crate::config::BackupSection::use_bundled_ytdlp`] is enabled, all
//! yt-dlp invocations use a binary under `~/.local/share/yt-offline/bin/`
//! instead of the system PATH. A bundled `deno` lives alongside so yt-dlp can
//! evaluate the JavaScript signature-deciphering code YouTube serves with the
//! player — without depending on a system-wide JS runtime.
//!
//! Both binaries are downloaded on demand from the official GitHub releases by
//! [`install_command`], which returns a shell pipeline that curls and unpacks
//! them. The pipeline is run as a regular yt-dlp [`crate::downloader::Job`] so
//! the user sees progress in the same UI.

use std::path::PathBuf;
use std::process::Command;

/// Directory holding the bundled binaries (`yt-dlp`, `deno`).
pub fn bundled_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".local").join("share").join("yt-offline").join("bin")
}

/// File name of the yt-dlp executable on this platform.
fn ytdlp_filename() -> &'static str {
    if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" }
}

/// Full path the bundled yt-dlp binary should live at.
pub fn bundled_ytdlp_path() -> PathBuf {
    bundled_dir().join(ytdlp_filename())
}

/// Returns the yt-dlp invocation target for [`std::process::Command::new`].
///
/// With `use_bundled = true` returns the absolute path to the bundled binary
/// (even if it does not yet exist — yt-dlp simply fails to launch and the
/// error surfaces in the job log, prompting the user to click Update).
/// Otherwise returns the bare `"yt-dlp"` string so the system PATH is used.
pub fn ytdlp_invocation(use_bundled: bool) -> PathBuf {
    if use_bundled {
        bundled_ytdlp_path()
    } else {
        PathBuf::from("yt-dlp")
    }
}

/// True if the bundled yt-dlp binary currently exists on disk.
pub fn bundled_installed() -> bool {
    bundled_ytdlp_path().exists()
}

/// Defensively re-apply `+x` to every regular file inside [`bundled_dir`].
///
/// Called before each bundled-mode invocation to guard against the install
/// script's `chmod` step having been skipped (e.g. partial install) or the
/// executable bit having been stripped by some other process.
///
/// No-op on Windows (executability is determined by the `.exe` suffix there).
pub fn ensure_bundled_executable() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = bundled_dir();
        let Ok(entries) = std::fs::read_dir(&dir) else { return };
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_file() { continue; }
            if let Ok(meta) = entry.metadata() {
                let mut perms = meta.permissions();
                let mode = perms.mode();
                let want = mode | 0o111;
                if mode != want {
                    perms.set_mode(want);
                    let _ = std::fs::set_permissions(&p, perms);
                }
            }
        }
    }
}

/// GitHub release asset name for yt-dlp on this platform.
fn ytdlp_asset() -> &'static str {
    if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else if cfg!(target_os = "macos") {
        "yt-dlp_macos"
    } else {
        "yt-dlp_linux"
    }
}

/// GitHub release asset name for deno on this platform.
fn deno_asset() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux",   "x86_64")  => "deno-x86_64-unknown-linux-gnu.zip",
        ("linux",   "aarch64") => "deno-aarch64-unknown-linux-gnu.zip",
        ("macos",   "x86_64")  => "deno-x86_64-apple-darwin.zip",
        ("macos",   "aarch64") => "deno-aarch64-apple-darwin.zip",
        ("windows", _)         => "deno-x86_64-pc-windows-msvc.zip",
        _                       => "deno-x86_64-unknown-linux-gnu.zip",
    }
}

/// Build a [`Command`] that installs or updates the bundled yt-dlp + deno
/// binaries by running a curl/unzip pipeline through `bash -c` (or PowerShell
/// on Windows). On success, both binaries are present in [`bundled_dir`] with
/// executable bit set.
pub fn install_command() -> Command {
    let dir = bundled_dir();
    let dir_str = dir.display().to_string();
    let ytdlp_url = format!(
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/{}",
        ytdlp_asset()
    );
    let deno_url = format!(
        "https://github.com/denoland/deno/releases/latest/download/{}",
        deno_asset()
    );
    let ytdlp_name = ytdlp_filename();

    #[cfg(windows)]
    {
        let script = format!(
            "$ErrorActionPreference='Stop'; \
             New-Item -ItemType Directory -Force -Path '{dir}' | Out-Null; \
             Write-Host '==> downloading yt-dlp'; \
             Invoke-WebRequest -Uri '{yurl}' -OutFile '{dir}\\{ybin}'; \
             Write-Host '==> downloading deno'; \
             Invoke-WebRequest -Uri '{durl}' -OutFile '{dir}\\deno.zip'; \
             Write-Host '==> extracting deno'; \
             Expand-Archive -Force '{dir}\\deno.zip' '{dir}'; \
             Remove-Item '{dir}\\deno.zip'; \
             Write-Host '==> versions'; & '{dir}\\{ybin}' --version; & '{dir}\\deno.exe' --version",
            dir = dir_str, yurl = ytdlp_url, durl = deno_url, ybin = ytdlp_name,
        );
        let mut cmd = Command::new("powershell");
        cmd.arg("-NoProfile").arg("-Command").arg(script);
        cmd
    }
    #[cfg(not(windows))]
    {
        // Download each file in the background and poll the growing file size
        // every 3 s so the job log shows visible progress during the transfer
        // instead of going silent for minutes.
        //
        // For yt-dlp we additionally fetch its published `SHA2-256SUMS` file
        // and verify the binary against it — that file is signed-by-presence
        // in the same GitHub release, so any tampering would have to compromise
        // both URLs to slip a bad binary through. Deno doesn't publish a
        // similarly convenient single-file checksum manifest, so we just print
        // its SHA-256 for visual inspection.
        let sums_url = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS";
        let ytdlp_asset_name = ytdlp_asset();
        let script = format!(
            r#"set -e
command -v curl     >/dev/null || {{ echo 'error: curl not installed';     exit 1; }}
command -v unzip    >/dev/null || {{ echo 'error: unzip not installed';    exit 1; }}
command -v sha256sum >/dev/null || {{ echo 'error: sha256sum not installed'; exit 1; }}
mkdir -p '{dir}'

# ── yt-dlp ──────────────────────────────────────────────────────────────────
echo '==> downloading yt-dlp'
curl -fL --no-progress-meter --connect-timeout 30 --max-time 600 --retry 3 \
  -o '{dir}/{ybin}' '{yurl}' &
DLPID=$!
while kill -0 $DLPID 2>/dev/null; do
  sleep 3
  SZ=$(wc -c < '{dir}/{ybin}' 2>/dev/null || echo 0)
  echo "    yt-dlp: $SZ bytes received..."
done
wait $DLPID
echo "    done: $(wc -c < '{dir}/{ybin}') bytes"

echo '==> verifying yt-dlp SHA-256'
curl -fL --no-progress-meter --connect-timeout 30 --max-time 60 \
  -o '{dir}/yt-dlp.sums' '{sums}'
EXPECTED=$(awk -v n="{yasset}" '$2==n {{ print $1 }}' '{dir}/yt-dlp.sums' | head -n1)
ACTUAL=$(sha256sum '{dir}/{ybin}' | awk '{{ print $1 }}')
rm '{dir}/yt-dlp.sums'
if [ -z "$EXPECTED" ]; then
  echo "    warn: no checksum entry for {yasset} in SHA2-256SUMS"
  echo "    actual SHA-256: $ACTUAL"
elif [ "$EXPECTED" != "$ACTUAL" ]; then
  echo "    error: SHA-256 mismatch"
  echo "    expected: $EXPECTED"
  echo "    actual:   $ACTUAL"
  rm -f '{dir}/{ybin}'
  exit 1
else
  echo "    ok: $ACTUAL"
fi
chmod +x '{dir}/{ybin}'

# ── deno ────────────────────────────────────────────────────────────────────
echo '==> downloading deno'
curl -fL --no-progress-meter --connect-timeout 30 --max-time 600 --retry 3 \
  -o '{dir}/deno.zip' '{durl}' &
DLPID=$!
while kill -0 $DLPID 2>/dev/null; do
  sleep 3
  SZ=$(wc -c < '{dir}/deno.zip' 2>/dev/null || echo 0)
  echo "    deno.zip: $SZ bytes received..."
done
wait $DLPID
echo "    done: $(wc -c < '{dir}/deno.zip') bytes"
echo "    deno.zip SHA-256: $(sha256sum '{dir}/deno.zip' | awk '{{ print $1 }}')"

echo '==> extracting deno'
unzip -o '{dir}/deno.zip' -d '{dir}'
chmod +x '{dir}/deno'
rm '{dir}/deno.zip'

echo '==> versions'
'{dir}/{ybin}' --version
'{dir}/deno' --version
echo '==> done'"#,
            dir = dir_str, yurl = ytdlp_url, durl = deno_url, ybin = ytdlp_name,
            sums = sums_url, yasset = ytdlp_asset_name,
        );
        let mut cmd = Command::new("bash");
        cmd.arg("-c").arg(script);
        cmd
    }
}
