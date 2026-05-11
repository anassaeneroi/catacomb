# YouTube Backup

A modern desktop application for archiving YouTube channels using [yt-dlp](https://github.com/yt-dlp/yt-dlp). Browse your downloaded channels, manage downloads, and play videos with your preferred media player—all from a clean, user-friendly GUI.

## Features

- **Channel Browser**: Browse downloaded YouTube channels and videos with thumbnails
- **Download Queue**: Queue new channels or videos for download with a simple interface
- **Video Playback**: Play videos directly using your preferred video player (mpv, VLC, etc.)
- **Live Chat Archives**: Identify and manage downloaded live chat archives
- **Search**: Search across your entire video library
- **System Tray**: Minimize to system tray for quick access
- **Configurable**: Customize backup location and video player via `config.toml`
- **SQLite Database**: Efficient metadata storage and retrieval

## Requirements

- **Rust** 1.70+ (for building from source)
- **yt-dlp**: Required for downloading videos. Install via:
  - `pip install yt-dlp` (Python)
  - `apt install yt-dlp` (Debian/Ubuntu)
  - `brew install yt-dlp` (macOS)
  - Or [build from source](https://github.com/yt-dlp/yt-dlp#installation)
- **Video Player**: A command-line compatible player like:
  - mpv (default)
  - VLC
  - ffplay
  - Any player accepting a file path as argument

## Installation

### From Source

```bash
git clone https://codeberg.org/your-username/youtube-backup
cd youtube-backup
cargo build --release
```

The compiled binary will be at `target/release/youtube-backup`.

### Linux Desktop Integration

A `.desktop` file is included for easy launcher integration:

```bash
cp youtube-backup.desktop ~/.local/share/applications/
```

## Configuration

Create a `config.toml` file in the same directory as the binary:

```toml
[backup]
directory = "/path/to/your/video/library"

[player]
command = "mpv"  # or "vlc", "ffplay", etc.
```

### Configuration Options

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `backup.directory` | string | `./channels` | Where downloaded videos are stored |
| `player.command` | string | `mpv` | Command to launch video player |

## Usage

### Running the Application

```bash
./youtube-backup
```

Or from your application launcher after installing the `.desktop` file.

### Workflow

1. **Add a Channel**: 
   - Click the "Download" tab
   - Enter the YouTube channel URL (e.g., `https://www.youtube.com/@channelname`)
   - Optionally specify a custom directory
   - Click "Queue Download"

2. **Browse Downloads**:
   - Videos are automatically organized by channel
   - Browse channels in the left sidebar
   - Click a channel to view its videos
   - Videos display with thumbnails and metadata

3. **Play a Video**:
   - Click any video card
   - Select "Play" to launch it in your configured player

4. **Search**:
   - Use the search bar to find videos by title
   - Results update in real-time across your entire library

5. **View Downloads**:
   - Toggle the "Show Downloads" button to monitor active downloads
   - Tracks download progress for queued jobs

## Project Structure

```
youtube-backup/
├── src/
│   ├── main.rs          # Application entry point
│   ├── app.rs           # Main GUI and event loop
│   ├── downloader.rs    # yt-dlp integration
│   ├── library.rs       # Channel and video scanning
│   ├── config.rs        # Configuration loading
│   ├── database.rs      # SQLite metadata storage
│   └── tray.rs          # System tray integration
├── config.toml          # Configuration template
├── Cargo.toml           # Rust dependencies
└── youtube-backup.desktop # Linux launcher entry
```

## Dependencies

- **eframe/egui**: Fast, native GUI framework for Rust
- **rusqlite**: SQLite database bindings (bundled)
- **serde/toml**: Configuration file parsing
- **image**: Thumbnail decoding (JPEG, PNG, WebP)
- **tray-icon**: System tray integration

## Building and Packaging

### Release Build

```bash
cargo build --release
```

### AUR (Arch Linux)

A `PKGBUILD` is included for Arch Linux:

```bash
makepkg -si
```

## Troubleshooting

### yt-dlp Not Found
Ensure `yt-dlp` is installed and in your `$PATH`:
```bash
which yt-dlp
```

### Videos Won't Play
Check that your configured player command is correct in `config.toml` and installed on your system.

### Thumbnails Not Loading
Verify the image dependencies are compiled correctly. Supported formats: JPEG, PNG, WebP.

### Database Issues
Delete `backup.db` (in the parent directory of your video library) and rescan:
```bash
rm backup.db
```
Restart the application to rebuild the index.

## License

See [LICENSE](LICENSE) file for details.

## Contributing

Contributions welcome! Feel free to open issues or submit pull requests.
