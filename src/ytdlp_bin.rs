//! Management of the bundled yt-dlp + deno binaries.
//!
//! When [`crate::config::BackupSection::use_bundled_ytdlp`] is enabled the
//! app invokes its own yt-dlp instead of whatever's on PATH. To get the
//! full feature set — most importantly `curl_cffi`-backed `--impersonate`
//! support — we install yt-dlp into a self-contained Python virtualenv
//! under `~/.local/share/yt-offline/venv/`. A bundled `deno` lives at
//! `~/.local/share/yt-offline/bin/deno` so yt-dlp can evaluate the
//! JavaScript signature-deciphering code YouTube serves with the player.
//!
//! Layout:
//! ```text
//! ~/.local/share/yt-offline/
//!   bin/                 ← prepended to PATH so yt-dlp finds deno
//!     deno
//!   venv/
//!     bin/yt-dlp         ← the real entry point (or Scripts\yt-dlp.exe on Windows)
//!     lib/python*/site-packages/{yt_dlp, curl_cffi, ...}
//! ```
//!
//! [`install_command`] returns the shell pipeline that builds this tree.
//! It's run as a regular [`crate::downloader::Job`] so the user sees
//! progress in the same UI as their other downloads.

use std::path::PathBuf;
use std::process::Command;

/// Root directory holding everything bundled — venv + bin.
pub fn bundled_root() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".local").join("share").join("yt-offline")
}

/// Directory holding non-Python binaries (currently just `deno`). Also the
/// path we prepend to `PATH` when spawning yt-dlp so it can locate deno.
pub fn bundled_dir() -> PathBuf {
    bundled_root().join("bin")
}

/// Root of the Python virtualenv that hosts the bundled yt-dlp install.
pub fn bundled_venv() -> PathBuf {
    bundled_root().join("venv")
}

/// Full path to the bundled yt-dlp entry point. On Unix this lives at
/// `venv/bin/yt-dlp`; on Windows it's `venv/Scripts/yt-dlp.exe`.
pub fn bundled_ytdlp_path() -> PathBuf {
    let venv = bundled_venv();
    if cfg!(windows) {
        venv.join("Scripts").join("yt-dlp.exe")
    } else {
        venv.join("bin").join("yt-dlp")
    }
}

/// Returns the yt-dlp invocation target for [`std::process::Command::new`].
///
/// With `use_bundled = true` returns the absolute path to the venv-installed
/// yt-dlp (even if it doesn't yet exist — the spawn error surfaces in the
/// job log and prompts the user to click Install). Otherwise returns the
/// bare `"yt-dlp"` string so the system PATH is used.
pub fn ytdlp_invocation(use_bundled: bool) -> PathBuf {
    if use_bundled {
        bundled_ytdlp_path()
    } else {
        PathBuf::from("yt-dlp")
    }
}

/// True if the bundled yt-dlp has been installed.
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
        // The venv's `bin/` already gets +x via pip; we only need to
        // re-apply on the extras we placed in `bundled_dir/`.
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

/// Build a [`Command`] that installs (or upgrades) the bundled yt-dlp into
/// a Python virtualenv with `curl_cffi` (for `--impersonate` support) and
/// the rest of `yt-dlp[default]`, plus a sibling `deno` for JS deciphering.
///
/// Runs through `bash -c` on Unix and PowerShell on Windows.
pub fn install_command() -> Command {
    let root = bundled_root();
    let root_str = root.display().to_string();
    let bin_str = bundled_dir().display().to_string();
    let venv_str = bundled_venv().display().to_string();
    let deno_url = format!(
        "https://github.com/denoland/deno/releases/latest/download/{}",
        deno_asset()
    );

    #[cfg(windows)]
    {
        let script = format!(
            "$ErrorActionPreference='Stop'; \
             New-Item -ItemType Directory -Force -Path '{root}' | Out-Null; \
             New-Item -ItemType Directory -Force -Path '{bin}' | Out-Null; \
             $py = (Get-Command py -ErrorAction SilentlyContinue) ?? (Get-Command python -ErrorAction SilentlyContinue); \
             if (-not $py) {{ Write-Error 'python is not installed'; exit 1 }}; \
             if (-not (Test-Path '{venv}\\Scripts\\python.exe')) {{ \
               Write-Host '==> creating Python venv'; \
               & $py.Source -m venv '{venv}'; \
             }}; \
             Write-Host '==> installing yt-dlp + curl_cffi'; \
             & '{venv}\\Scripts\\python.exe' -m pip install --upgrade pip; \
             & '{venv}\\Scripts\\python.exe' -m pip install --upgrade 'yt-dlp[default]' curl_cffi; \
             Write-Host '==> downloading deno'; \
             Invoke-WebRequest -Uri '{durl}' -OutFile '{bin}\\deno.zip'; \
             Write-Host '==> extracting deno'; \
             Expand-Archive -Force '{bin}\\deno.zip' '{bin}'; \
             Remove-Item '{bin}\\deno.zip'; \
             Write-Host '==> versions'; \
             & '{venv}\\Scripts\\yt-dlp.exe' --version; \
             & '{bin}\\deno.exe' --version; \
             Write-Host '==> done'",
            root = root_str, bin = bin_str, venv = venv_str, durl = deno_url,
        );
        let mut cmd = Command::new("powershell");
        cmd.arg("-NoProfile").arg("-Command").arg(script);
        cmd
    }
    #[cfg(not(windows))]
    {
        // Set up yt-dlp inside a venv so we get the full pip-installable
        // distribution — including `curl_cffi`, which is what enables
        // `--impersonate` support. The standalone PyInstaller binary we
        // previously shipped lacks curl_cffi and errors out on impersonate
        // targets. The venv strategy is also what yt-dlp's own docs
        // recommend for getting a "full" install.
        //
        // Pip-installed packages come with their own SHA-256 checks via
        // PyPI's metadata, so we skip the manual checksum step we used to
        // do for the standalone binary.
        //
        // Deno is unchanged: it's a single static binary that we drop into
        // `bin/` alongside the (unused) directory; yt-dlp finds it via PATH
        // when we prepend `bin/` in [`crate::downloader::Downloader::spawn_job`].
        let script = format!(
            r#"set -e
command -v curl  >/dev/null || {{ echo 'error: curl not installed';  exit 1; }}
command -v unzip >/dev/null || {{ echo 'error: unzip not installed'; exit 1; }}
if ! command -v python3 >/dev/null; then
  echo 'error: python3 is not installed.'
  echo '       Install it from your distro package manager and try again.'
  exit 1
fi
# Detect whether the venv module is actually usable. On Debian/Ubuntu it
# ships in the `python3-venv` package and fails noisily when missing.
if ! python3 -c 'import venv' 2>/dev/null; then
  echo 'error: the Python "venv" module is not installed.'
  echo '       Install python3-venv (Debian/Ubuntu) or the equivalent.'
  exit 1
fi

mkdir -p '{root}'
mkdir -p '{bin}'

# Remove any legacy PyInstaller binary from the prior install layout so
# bundled_ytdlp_path()'s new venv-based path is the only one resolvable.
rm -f '{bin}/yt-dlp'

if [ ! -x '{venv}/bin/python' ]; then
  echo '==> creating Python venv at {venv}'
  python3 -m venv '{venv}'
fi

echo '==> upgrading pip in venv'
'{venv}/bin/python' -m pip install --upgrade --quiet pip

echo '==> installing yt-dlp[default] + curl_cffi (this fetches a few packages)'
'{venv}/bin/python' -m pip install --upgrade --quiet --progress-bar off 'yt-dlp[default]' curl_cffi

# ── deno ────────────────────────────────────────────────────────────────────
echo '==> downloading deno'
curl -fL --no-progress-meter --connect-timeout 30 --max-time 600 --retry 3 \
  -o '{bin}/deno.zip' '{durl}' &
DLPID=$!
while kill -0 $DLPID 2>/dev/null; do
  sleep 3
  SZ=$(wc -c < '{bin}/deno.zip' 2>/dev/null || echo 0)
  echo "    deno.zip: $SZ bytes received..."
done
wait $DLPID
echo "    done: $(wc -c < '{bin}/deno.zip') bytes"
if command -v sha256sum >/dev/null; then
  echo "    deno.zip SHA-256: $(sha256sum '{bin}/deno.zip' | awk '{{ print $1 }}')"
fi

echo '==> extracting deno'
unzip -o '{bin}/deno.zip' -d '{bin}'
chmod +x '{bin}/deno'
rm '{bin}/deno.zip'

echo '==> versions'
'{venv}/bin/yt-dlp' --version
'{bin}/deno' --version
echo '==> verifying curl_cffi (impersonation) is available'
if '{venv}/bin/yt-dlp' --list-impersonate-targets 2>/dev/null | grep -qi 'chrome'; then
  echo '    ok: impersonation targets available'
else
  echo '    warn: curl_cffi did not load; --impersonate will be skipped'
fi
echo '==> done'"#,
            root = root_str, bin = bin_str, venv = venv_str, durl = deno_url,
        );
        let mut cmd = Command::new("bash");
        cmd.arg("-c").arg(script);
        cmd
    }
}
