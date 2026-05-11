# yt-offline

A desktop app for archiving YouTube channels using [yt-dlp](https://github.com/yt-dlp/yt-dlp). Browse your downloaded channels, manage downloads, and play videos — all from a native GUI.

## Features

- **Channel Browser** — Browse downloaded channels and videos with thumbnails
- **Smart Download Routing** — Paste any YouTube URL; channels, playlists, and single videos are automatically routed to the right folder
- **Playlist View** — Channels with playlist subdirectories show them in the sidebar
- **Themes** — Dark, Light, Dracula, Trans, and three Emo/Scene themes
- **Settings GUI** — Configure everything from inside the app
- **Video Playback** — Launch videos in mpv, VLC, or any player
- **Search** — Filter across your entire library in real time
- **System Tray** — Minimize to tray

## Building

### Arch Linux / Manjaro

A `PKGBUILD` is included:

```bash
git clone https://codeberg.org/anassaeneroi/yt-offline
cd yt-offline
makepkg -si
```

Or build manually:

```bash
sudo pacman -S --needed rust yt-dlp mpv
cargo build --release
sudo install -Dm755 target/release/yt-offline /usr/bin/yt-offline
```

---

### Debian / Ubuntu / Linux Mint

Install build dependencies:

```bash
sudo apt install \
  build-essential pkg-config curl git \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libssl-dev \
  libgtk-3-dev libayatana-appindicator3-dev
```

Install Rust (if not already installed):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

Install runtime dependencies:

```bash
sudo apt install yt-dlp mpv
```

Build and install:

```bash
git clone https://codeberg.org/anassaeneroi/yt-offline
cd yt-offline
cargo build --release
sudo install -Dm755 target/release/yt-offline /usr/bin/yt-offline
cp youtube-backup.desktop ~/.local/share/applications/yt-offline.desktop
```

---

### macOS

Install Xcode command line tools and Homebrew dependencies:

```bash
xcode-select --install
brew install yt-dlp mpv
```

Install Rust:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

Build:

```bash
git clone https://codeberg.org/anassaeneroi/yt-offline
cd yt-offline
cargo build --release
```

The binary is at `target/release/yt-offline`. Copy it wherever you like, or run it in place.

---

### Windows

1. Install [Rust](https://rustup.rs) — accept the default MSVC toolchain when prompted
2. Install [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the **Desktop development with C++** workload
3. Install yt-dlp and mpv:

   ```powershell
   winget install yt-dlp.yt-dlp
   winget install mpv.net
   ```

4. Clone and build:

   ```powershell
   git clone https://codeberg.org/anassaeneroi/yt-offline
   cd yt-offline
   cargo build --release
   ```

The binary is at `target\release\yt-offline.exe`. Copy it wherever you like and run it.

> **Note:** The first build takes a while — Rust compiles all dependencies from scratch.

---

## Configuration

On first run, create a `config.toml` next to the binary (or edit it from the Settings button inside the app):

```toml
[backup]
directory = "/path/to/your/video/library"

[player]
command = "mpv"

[ui]
theme = "dark"
```

### Options

| Setting | Default | Description |
| ------- | ------- | ----------- |
| `backup.directory` | `./channels` | Where downloaded videos are stored |
| `player.command` | `mpv` | Command used to launch the video player |
| `ui.theme` | `dark` | `dark`, `light`, `dracula`, `trans`, `emo-nocturnal`, `emo-coffin`, `emo-scene-queen` |

---

## Usage

1. **Download a channel/video/playlist** — click **⬇ Downloads**, paste a YouTube URL. The app detects the URL type and routes it automatically:
   - Channel URL → `channels/<handle>/`
   - Single video → `channels/<channel-name>/`
   - Playlist → `channels/<channel-name>/<playlist-name>/`

2. **Browse your library** — channels appear in the left sidebar. If a channel has playlist subdirectories they show as collapsible sub-items.

3. **Play a video** — click **▶ Play** on any video card, or double-click the thumbnail.

4. **Change settings** — click **⚙ Settings** to change the backup directory, player, or theme without editing the file.

---

## Troubleshooting

**`yt-dlp` not found**
Make sure it's installed and on your `$PATH`. On Windows, restart your terminal after install.

**Build fails on Debian/Ubuntu with missing headers**
Make sure you installed all packages in the build dependencies block above, especially `libayatana-appindicator3-dev` and `libgtk-3-dev`.

**Build fails on Windows with linker errors**
Make sure Visual Studio Build Tools are installed with the C++ workload selected, not just the base tools.

**Videos won't play**
Check `player.command` in `config.toml` (or Settings). The command must accept a file path as its last argument.

**Thumbnails not loading**
Supported formats: JPEG, PNG, WebP. Other formats are silently skipped.

---

## Project Structure

```text
src/
  main.rs        entry point
  app.rs         UI and main loop
  downloader.rs  yt-dlp integration and URL detection
  library.rs     channel/playlist/video scanner
  config.rs      config file loading and saving
  theme.rs       all colour themes
  database.rs    SQLite (reserved for future use)
  tray.rs        system tray
```

## License

See [LICENSE](LICENSE).
