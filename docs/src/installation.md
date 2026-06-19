# Installation

Catacomb is a single Rust binary. You can install a prebuilt package,
build from source, or grab the AppImage.

## Runtime dependencies

Whichever way you install, these are invoked as subprocesses at runtime:

- **yt-dlp** — the download engine. You can use the system one *or* let
  catacomb manage a bundled copy (see [First run](./first-run.md)).
- **ffmpeg** — muxing, format conversion, on-the-fly transcode for the
  web player.
- **mpv** — the default desktop player (any player taking a file path
  works; set it in Settings).
- **xdg-utils** — `xdg-open` for "Show in file manager".

## Prebuilt packages (Linux)

Releases attach `.deb`, `.rpm`, and `.AppImage` artifacts.

```sh
# Debian / Ubuntu / Mint
sudo apt install ./catacomb_*_amd64.deb

# Fedora / RHEL / openSUSE   (ffmpeg via RPM Fusion)
sudo dnf install ./catacomb-*.x86_64.rpm

# Any Linux — AppImage
chmod +x catacomb-*-x86_64.AppImage
./catacomb-*-x86_64.AppImage
```

## Arch / CachyOS / Manjaro

A `PKGBUILD` ships in the repo root. Build it from a **clean** directory:

```sh
mkdir build && cd build
cp /path/to/repo/PKGBUILD .
makepkg -si
```

For repeated builds after pulling new commits, always pass `-C`
(cleanbuild) so makepkg re-checks out the latest source instead of
reusing a stale cached clone.

## From source

```sh
# Debian/Ubuntu build deps
sudo apt install build-essential pkg-config curl git python3-venv \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libssl-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

git clone https://codeberg.org/anassaeneroi/catacomb
cd catacomb
cargo build --release
./target/release/catacomb           # desktop GUI
./target/release/catacomb --web 8080  # headless web server
```

`python3-venv` is only needed for the bundled-yt-dlp install path; skip
it if you'll always use system yt-dlp.

## Windows / macOS

Windows is available as a release zip when built through
`scripts/package.sh win` or tag-release CI. It includes `catacomb.exe`,
the license, and a short runtime-dependency README; install `yt-dlp`,
`ffmpeg`, and `mpv` / `mpv.net` on PATH.

macOS has a local osxcross packaging path (`scripts/package.sh mac`) that
assembles an unsigned `.app` zip when you provide an Apple SDK locally.
It is not built in public CI because the SDK cannot be redistributed, and
codesigning/notarization remain follow-ups. See
[packaging](./packaging.md) and the [architecture notes](./architecture.md#platform-support).
