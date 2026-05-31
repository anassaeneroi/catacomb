# Packaging yt-offline

This document covers building distributable packages. The one-liner:

```sh
scripts/package.sh all     # .deb + .rpm + .AppImage into dist/
scripts/package.sh deb     # just the .deb
scripts/package.sh rpm     # just the .rpm
scripts/package.sh appimage
```

The script builds the release binary once and reuses it for every
format. Output lands in `dist/` (gitignored).

## Linux formats

### .deb (Debian / Ubuntu / Mint)

Built by [`cargo-deb`](https://github.com/kornelski/cargo-deb), driven by
`[package.metadata.deb]` in `Cargo.toml`. The script installs cargo-deb
on demand. Runtime deps declared: `yt-dlp`, `ffmpeg`, `mpv`, `xdg-utils`,
`libxcb1`, `libc6` (and `libnotify4` recommended).

```sh
scripts/package.sh deb
sudo apt install ./dist/yt-offline_*_amd64.deb
```

### .rpm (Fedora / RHEL / openSUSE)

Built by [`cargo-generate-rpm`](https://github.com/cat-in-136/cargo-generate-rpm),
driven by `[package.metadata.generate-rpm]`. It packages the
already-built (and release-profile-stripped) binary.

```sh
scripts/package.sh rpm
sudo dnf install ./dist/yt-offline-*.x86_64.rpm
```

Note: `ffmpeg` on Fedora lives in [RPM Fusion](https://rpmfusion.org/);
the dependency is declared but the user may need that repo enabled.

### AppImage (any Linux)

A hand-rolled AppDir + [`appimagetool`](https://github.com/AppImage/appimagetool)
(downloaded to `dist/tools/` on first run). The image bundles the GUI
binary and its shared-library closure only — `yt-dlp`/`ffmpeg`/`mpv` are
still expected on the host PATH, same as the package deps. Bundling them
would balloon the image and tangle the licensing.

```sh
scripts/package.sh appimage
chmod +x dist/yt-offline-*-x86_64.AppImage
./dist/yt-offline-*-x86_64.AppImage
```

## Arch Linux

Use the `PKGBUILD` in the repo root (not this script). It builds from a
fresh git clone, so run it from a clean directory:

```sh
mkdir build && cd build
cp /path/to/repo/PKGBUILD .
makepkg -si
```

For repeated builds after pushing new commits, always pass `-C`
(cleanbuild) so makepkg re-checks out the latest source instead of
reusing a stale cached clone.

## Windows — experimental

Windows is **not** currently a first-class target. Two Linux-only
dependencies block a clean `--target x86_64-pc-windows-gnu` build:

- **`ksni`** (system tray) — talks to the freedesktop StatusNotifierItem
  D-Bus spec, which doesn't exist on Windows. Needs replacing with
  `tray-icon` behind `#[cfg(windows)]`, or stubbing `src/tray.rs` to a
  no-op on non-Unix.
- **`rfd` xdg-portal backend** — the file picker uses the XDG desktop
  portal. The `rfd` crate does support a native Windows backend, but the
  feature flags in `Cargo.toml` would need to be made target-conditional.

Once those are addressed, the rest (eframe, axum, rusqlite-bundled) is
already cross-platform — `bundled_ytdlp_path()` and friends already have
`cfg!(windows)` branches. The path to a Windows `.exe`/`.msi` (via
[`cargo-wix`](https://github.com/volks73/cargo-wix)) is then mechanical.

Tracked as a follow-up; PRs welcome.

## macOS

Same shape as Windows: the tray needs a macOS backend. eframe runs fine
on macOS otherwise. A `.app` bundle + `.dmg` would follow once the tray
is abstracted behind a trait with per-OS implementations.

## CI

`.forgejo/workflows/release.yml` runs `scripts/package.sh all` on every
pushed tag (`v*`) and attaches the resulting `.deb`/`.rpm`/`.AppImage` to
the Codeberg release. See that file for the runner setup.
