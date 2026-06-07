# Packaging

Build distributable Linux packages with one script:

```sh
scripts/package.sh all       # .deb + .rpm + .AppImage → dist/
scripts/package.sh deb       # just the .deb
scripts/package.sh rpm
scripts/package.sh appimage
```

It builds the release binary once and reuses it for every format,
installing `cargo-deb` / `cargo-generate-rpm` on demand and downloading
`appimagetool` to `dist/tools/` on first AppImage build. Per-format
failures are isolated and summarized at the end. Output (gitignored)
lands in `dist/`.

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

`.forgejo/workflows/release.yml` runs `scripts/package.sh all` on every
pushed `v*` tag and attaches the artifacts to the Codeberg release.
`.forgejo/workflows/test.yml` runs the full test suite on every push.

The repo's [`docs/PACKAGING.md`](https://codeberg.org/anassaeneroi/yt-offline/src/branch/main/docs/PACKAGING.md)
has the per-distro install commands and the Windows/macOS status in full.
