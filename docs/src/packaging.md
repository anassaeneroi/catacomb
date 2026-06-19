# Packaging

Build distributable Linux packages with one script:

```sh
scripts/package.sh all       # Linux packages + Windows zip when toolchains exist
scripts/package.sh deb       # just the .deb
scripts/package.sh rpm
scripts/package.sh appimage
scripts/package.sh win       # Windows .zip via mingw-w64
scripts/package.sh mac       # unsigned macOS .app .zip via local osxcross SDK
```

It builds the release binary once and reuses it for every format,
installing `cargo-deb` / `cargo-generate-rpm` on demand and downloading
`appimagetool` to `dist/tools/` on first AppImage build. Windows is included
when the mingw target/toolchain is present; macOS is local-only because
osxcross needs an Apple SDK. Per-format failures are isolated and summarized
at the end. Output (gitignored) lands in `dist/`.

## Formats

- **`.deb`** — built by [cargo-deb](https://github.com/kornelski/cargo-deb)
  from `[package.metadata.deb]` in `Cargo.toml`.
- **`.rpm`** — built by
  [cargo-generate-rpm](https://github.com/cat-in-136/cargo-generate-rpm)
  from `[package.metadata.generate-rpm]`. (`ffmpeg` on Fedora is in
  RPM Fusion.)
- **AppImage** — a hand-rolled AppDir + appimagetool. Bundles the GUI
  binary's shared-library closure only; `yt-dlp`/`ffmpeg`/`mpv` stay host
  PATH deps, same as the package declarations.
- **Arch** — use the repo's `PKGBUILD` (not this script); run `makepkg`
  from a clean directory.

## CI

The repo ships `.forgejo/workflows/` definitions (`test.yml`,
`release.yml`), but Codeberg executes Woodpecker rather than Forgejo
Actions — so they don't run there without a self-hosted runner. Until
then, build packages locally with `scripts/package.sh` and publish docs
with `scripts/publish-docs.sh`.

The repo's [`docs/PACKAGING.md`](https://codeberg.org/anassaeneroi/catacomb/src/branch/main/docs/PACKAGING.md)
has the per-distro install commands and the Windows/macOS status in full.
