# Packaging Catacomb

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
sudo apt install ./dist/catacomb_*_amd64.deb
```

### .rpm (Fedora / RHEL / openSUSE)

Built by [`cargo-generate-rpm`](https://github.com/cat-in-136/cargo-generate-rpm),
driven by `[package.metadata.generate-rpm]`. It packages the
already-built (and release-profile-stripped) binary.

```sh
scripts/package.sh rpm
sudo dnf install ./dist/catacomb-*.x86_64.rpm
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
chmod +x dist/catacomb-*-x86_64.AppImage
./dist/catacomb-*-x86_64.AppImage
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

## Windows (.zip)

Windows cross-compiles cleanly from Linux — the formerly-blocking
Linux-only deps are target-gated in `Cargo.toml` (`ksni` and `rfd`'s
xdg-portal backend are `cfg(target_os = "linux")`; `tray::start` is a
no-op off Linux; `rfd` falls back to its native Win32 backend).

```sh
rustup target add x86_64-pc-windows-gnu
sudo apt install mingw-w64 zip          # or your distro's equivalent
scripts/package.sh win
```

This produces `dist/catacomb-<ver>-x86_64-windows.zip` containing
`catacomb.exe`, `LICENSE.txt`, and a `README.txt` listing the runtime
PATH deps (yt-dlp / ffmpeg / mpv). Release builds link as a GUI-subsystem
app (no console window); the binary reattaches to the launching terminal
at runtime (`attach_windows_console` in `main.rs`) so `catacomb.exe
--web 8080` from PowerShell prints its logs, while a double-click stays
windowless.

`scripts/package.sh all` includes the Windows zip automatically when the
target + mingw + zip are all present, and silently skips it otherwise.

## macOS (.app .zip)

macOS cross-compiles via [osxcross](https://github.com/tpoechtrager/osxcross),
which needs Apple's macOS SDK (extracted from Xcode — its license does not
permit redistribution, which is why this build is **local-only and not in
CI**). The crypto stack is `ring`, which builds against the osxcross SDK
without trouble.

One-time setup: build osxcross with a macOS SDK and put its
`target/bin` on your `PATH` (this provides the `oa64-clang` / `o64-clang`
wrapper compilers). Then:

```sh
rustup target add aarch64-apple-darwin   # or x86_64-apple-darwin
MAC_ARCH=arm64 scripts/package.sh mac     # arm64 (default) | x86_64
```

This cross-compiles, assembles an unsigned `catacomb.app`
(`Info.plist` + the Mach-O binary; an `.icns` icon too if `png2icns` is
available), and zips it to `dist/catacomb-<ver>-<arch>-macos.zip`. The
`.app` is **not codesigned or notarized**, so on the target Mac it must be
opened the first time via right-click → Open, or cleared with `xattr -dr
com.apple.quarantine catacomb.app`. Runtime deps (yt-dlp / ffmpeg / mpv)
are expected on PATH as on the other platforms.

A `.dmg` and codesigning/notarization (and a `tray-icon`-based macOS tray)
are follow-ups; the tray is currently a no-op off Linux. A **MacPorts**
port (a `Portfile` building from source, like the Arch `PKGBUILD`) is a
possible future native-distribution path — unscheduled.

## CI

`.forgejo/workflows/release.yml` runs `scripts/package.sh all` on every
pushed tag (`v*`) and attaches the resulting artifacts to the Codeberg
release. The Linux container also installs `mingw-w64` + `zip` + the
`x86_64-pc-windows-gnu` target, so the **Windows zip is built in CI**
alongside the `.deb`/`.rpm`/`.AppImage`. The **macOS zip is not in CI**
(the SDK can't be hosted in a public image) — build it locally per the
section above, or add a GitHub Actions `macos-latest` job which has Xcode
and can also codesign/notarize.
