# Installation

yt-offline is a single Rust binary. You can install a prebuilt package,
build from source, or grab the AppImage.

## Runtime dependencies

Whichever way you install, these are invoked as subprocesses at runtime:

- **yt-dlp** — the download engine. You can use the system one *or* let
  yt-offline manage a bundled copy (see [First run](./first-run.md)).
- **ffmpeg** — muxing, format conversion, on-the-fly transcode for the
  web player.
- **mpv** — the default desktop player (any player taking a file path
  works; set it in Settings).
- **xdg-utils** — `xdg-open` for "Show in file manager".

## Prebuilt packages (Linux)

Releases attach `.deb`, `.rpm`, and `.AppImage` artifacts.

```sh
# Debian / Ubuntu / Mint
sudo apt install ./yt-offline_*_amd64.deb

# Fedora / RHEL / openSUSE   (ffmpeg via RPM Fusion)
sudo dnf install ./yt-offline-*.x86_64.rpm

# Any Linux — AppImage
chmod +x yt-offline-*-x86_64.AppImage
./yt-offline-*-x86_64.AppImage
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

git clone https://codeberg.org/anassaeneroi/yt-offline
cd yt-offline
cargo build --release
./target/release/yt-offline           # desktop GUI
./target/release/yt-offline --web 8080  # headless web server
```

`python3-venv` is only needed for the bundled-yt-dlp install path; skip
it if you'll always use system yt-dlp.

## Windows / macOS

Not first-class yet. The Linux-only system tray (`ksni`) and file dialog
(`rfd` xdg-portal) need per-OS backends before a clean cross-build; the
rest of the stack already compiles. See the
[architecture notes](./architecture.md#platform-support).
