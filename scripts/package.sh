#!/usr/bin/env bash
# Build distributable packages for catacomb.
#
# Usage:
#   scripts/package.sh [deb|rpm|appimage|win|mac|all]
#
# With no argument, builds everything that's buildable on the current host.
# Output lands in dist/. Each format is independent — a failure in one
# doesn't abort the others (the script reports a summary at the end).
#
# Prerequisites are installed on demand where possible:
#   - .deb     → cargo-deb           (cargo install cargo-deb)
#   - .rpm     → cargo-generate-rpm  (cargo install cargo-generate-rpm)
#   - AppImage → appimagetool        (downloaded to dist/tools/ if missing)
#   - win .zip → x86_64-pc-windows-gnu target + mingw-w64 gcc + zip
#   - mac .zip → {aarch64,x86_64}-apple-darwin target + osxcross + zip
#
# The Linux release binary is built once up front and reused by the Linux
# formats; the Windows/macOS .zips cross-compile separate targets on demand.
# `all` skips the Windows and macOS zips unless their cross toolchains are
# present, so a plain `all` on a stock Linux box still succeeds.
#
# macOS notes: cross-compiling for Apple targets needs osxcross
# (https://github.com/tpoechtrager/osxcross) built with a macOS SDK, and its
# wrapper compilers (oa64-clang / o64-clang) on PATH. Set MAC_ARCH=arm64
# (default) or x86_64. The result is an unsigned catacomb.app inside a .zip;
# it is *not* codesigned or notarized, so on the target Mac it must be opened
# via right-click → Open (or `xattr -dr com.apple.quarantine`) the first time.
# See docs/PACKAGING.md.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DIST="$ROOT/dist"
TOOLS="$DIST/tools"
mkdir -p "$DIST" "$TOOLS"

# Pull version + name straight from Cargo.toml so packages stay in sync.
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
PKG="catacomb"
ARCH="$(uname -m)"

say()  { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarn:\033[0m %s\n' "$*" >&2; }
err()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; }

WANT="${1:-all}"
declare -A RESULT  # format → "ok <path>" | "skip <reason>" | "fail <reason>"

# ── Build the release binary once ────────────────────────────────────────────
build_binary() {
    say "building release binary (this is the slow part)…"
    if ! cargo build --release; then
        err "cargo build failed — aborting, no packages can be made"
        exit 1
    fi
    if [[ ! -x target/release/$PKG ]]; then
        err "expected target/release/$PKG to exist after build"
        exit 1
    fi
}

# ── .deb ─────────────────────────────────────────────────────────────────────
build_deb() {
    say "building .deb"
    # Check via `cargo deb` rather than `command -v cargo-deb`: cargo
    # subcommands live in ~/.cargo/bin which may not be on PATH in CI,
    # but `cargo deb` resolves them through cargo itself.
    if ! cargo deb --help >/dev/null 2>&1; then
        say "installing cargo-deb…"
        cargo install cargo-deb || { RESULT[deb]="fail cargo-deb install failed"; return; }
    fi
    # --no-build: reuse the binary we already compiled above.
    if cargo deb --no-build --output "$DIST/"; then
        # cargo-deb names it ${PKG}_${VERSION}-1_${arch}.deb; find it.
        local out
        out="$(ls -t "$DIST"/${PKG}_*.deb 2>/dev/null | head -1)"
        RESULT[deb]="ok ${out:-$DIST}"
    else
        RESULT[deb]="fail cargo deb returned nonzero"
    fi
}

# ── .rpm ─────────────────────────────────────────────────────────────────────
build_rpm() {
    say "building .rpm"
    if ! cargo generate-rpm --help >/dev/null 2>&1; then
        say "installing cargo-generate-rpm…"
        cargo install cargo-generate-rpm || { RESULT[rpm]="fail cargo-generate-rpm install failed"; return; }
    fi
    # cargo-generate-rpm packages the already-built binary; we stripped it
    # via the release profile so an explicit strip step isn't needed.
    if cargo generate-rpm --output "$DIST/"; then
        local out
        out="$(ls -t "$DIST"/${PKG}-*.rpm 2>/dev/null | head -1)"
        RESULT[rpm]="ok ${out:-$DIST}"
    else
        RESULT[rpm]="fail cargo generate-rpm returned nonzero"
    fi
}

# ── AppImage ─────────────────────────────────────────────────────────────────
# Hand-rolled AppDir + appimagetool. We don't bundle the subprocess deps
# (yt-dlp / ffmpeg / mpv) — they're expected on PATH like the package deps,
# and bundling them would balloon the image and complicate licensing. The
# AppImage carries the GUI binary + its shared-lib closure only.
build_appimage() {
    say "building AppImage"
    local tool="$TOOLS/appimagetool"
    if [[ ! -x "$tool" ]]; then
        local tool_url="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${ARCH}.AppImage"
        say "downloading appimagetool…"
        if ! curl -fL --retry 3 -o "$tool" "$tool_url"; then
            RESULT[appimage]="fail could not download appimagetool"
            return
        fi
        chmod +x "$tool"
    fi

    local appdir="$DIST/${PKG}.AppDir"
    rm -rf "$appdir"
    mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications" "$appdir/usr/share/icons/hicolor/256x256/apps"

    install -Dm755 "target/release/$PKG" "$appdir/usr/bin/$PKG"
    install -Dm644 catacomb.desktop "$appdir/usr/share/applications/$PKG.desktop"
    # AppImage wants the desktop file + icon at the AppDir root too.
    cp catacomb.desktop "$appdir/$PKG.desktop"
    install -Dm644 icon.png "$appdir/usr/share/icons/hicolor/256x256/apps/$PKG.png"
    cp icon.png "$appdir/$PKG.png"

    # AppRun is the entry point. Exec the bundled binary, forwarding args.
    cat > "$appdir/AppRun" <<'APPRUN'
#!/usr/bin/env bash
HERE="$(dirname "$(readlink -f "${0}")")"
exec "$HERE/usr/bin/catacomb" "$@"
APPRUN
    chmod +x "$appdir/AppRun"

    local out="$DIST/${PKG}-${VERSION}-${ARCH}.AppImage"
    # ARCH env var tells appimagetool which arch to stamp into the runtime.
    if ARCH="$ARCH" "$tool" --no-appstream "$appdir" "$out" 2>&1; then
        RESULT[appimage]="ok $out"
    else
        RESULT[appimage]="fail appimagetool returned nonzero"
    fi
    rm -rf "$appdir"
}

# ── Windows .zip (cross-compiled via mingw-w64) ──────────────────────────────
# Like the AppImage we don't bundle the subprocess deps (yt-dlp / ffmpeg /
# mpv) — on Windows they're expected on PATH, and the README in the zip says
# so. The release build links a GUI-subsystem exe (no console window); the
# binary reattaches to the parent console at runtime for `--web`/CLI use
# (see attach_windows_console in main.rs).
WIN_TARGET="x86_64-pc-windows-gnu"
build_win() {
    say "building Windows .zip ($WIN_TARGET)"
    if ! rustup target list --installed 2>/dev/null | grep -qx "$WIN_TARGET"; then
        RESULT[win]="skip rust target $WIN_TARGET not installed (rustup target add $WIN_TARGET)"
        return
    fi
    if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
        RESULT[win]="skip mingw-w64 gcc not found (install mingw-w64)"
        return
    fi
    if ! command -v zip >/dev/null 2>&1; then
        RESULT[win]="skip zip not found"
        return
    fi
    if ! cargo build --release --target "$WIN_TARGET"; then
        RESULT[win]="fail cargo build --target $WIN_TARGET returned nonzero"
        return
    fi
    local exe="target/$WIN_TARGET/release/$PKG.exe"
    if [[ ! -f "$exe" ]]; then
        RESULT[win]="fail expected $exe after build"
        return
    fi

    local staging="$DIST/${PKG}-win"
    rm -rf "$staging"
    mkdir -p "$staging"
    install -m755 "$exe" "$staging/$PKG.exe"
    install -m644 LICENSE "$staging/LICENSE.txt"
    cat > "$staging/README.txt" <<README
Catacomb ${VERSION} — Windows build

Run the GUI by double-clicking catacomb.exe.

To run the headless web server, open PowerShell or Command Prompt in this
folder and run:

    .\\catacomb.exe --web 8080

then browse to http://localhost:8080. (Launched from a terminal the server
prints its logs to that terminal; double-clicked it runs windowless.)

Runtime requirements — these external tools must be installed and on your
PATH (the .exe shells out to them, it does not bundle them):

    yt-dlp    https://github.com/yt-dlp/yt-dlp   (the downloader engine)
    ffmpeg    https://ffmpeg.org                 (muxing / transcode / dedup)
    mpv       https://mpv.io                      (desktop playback; optional)

config.toml and cookies.txt are read from the working directory, the same
as on Linux. See the project README for configuration details.

Licensed under the GNU Affero General Public License v3 or later; see
LICENSE.txt. Source: https://codeberg.org/${PKG}
README

    local out="$DIST/${PKG}-${VERSION}-x86_64-windows.zip"
    rm -f "$out"
    # -j junks paths so the zip has the three files at its root.
    if (cd "$staging" && zip -q -j "$out" "$PKG.exe" LICENSE.txt README.txt); then
        RESULT[win]="ok $out"
    else
        RESULT[win]="fail zip returned nonzero"
    fi
    rm -rf "$staging"
}

# Whether to fold the Windows zip into a bare `all` run: only when the cross
# toolchain is actually present, so `all` on a stock Linux box still passes.
win_available() {
    rustup target list --installed 2>/dev/null | grep -qx "$WIN_TARGET" \
        && command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1 \
        && command -v zip >/dev/null 2>&1
}

# ── macOS .app .zip (cross-compiled via osxcross) ────────────────────────────
# Builds an unsigned catacomb.app for one Apple arch (MAC_ARCH=arm64 default,
# or x86_64) and zips it. Requires osxcross's wrapper compilers on PATH; the
# crypto stack is `ring`, which cross-builds against osxcross's SDK fine. Like
# the other formats we don't bundle yt-dlp/ffmpeg/mpv — the .app's README and
# Info.plist note they're expected on PATH. Skips cleanly if the toolchain is
# absent so `all` on a stock Linux box is unaffected.
mac_target_for() {  # arch → rust target triple
    case "$1" in
        arm64|aarch64) echo "aarch64-apple-darwin" ;;
        x86_64|x64)    echo "x86_64-apple-darwin" ;;
        *)             echo "" ;;
    esac
}
mac_wrapper_for() {  # arch → osxcross clang wrapper name
    case "$1" in
        arm64|aarch64) echo "oa64-clang" ;;
        x86_64|x64)    echo "o64-clang" ;;
        *)             echo "" ;;
    esac
}

mac_available() {
    local arch="${MAC_ARCH:-arm64}"
    local target wrapper
    target="$(mac_target_for "$arch")"; wrapper="$(mac_wrapper_for "$arch")"
    [[ -n "$target" ]] \
        && rustup target list --installed 2>/dev/null | grep -qx "$target" \
        && command -v "$wrapper" >/dev/null 2>&1 \
        && command -v zip >/dev/null 2>&1
}

build_mac() {
    local arch="${MAC_ARCH:-arm64}"
    local target wrapper
    target="$(mac_target_for "$arch")"; wrapper="$(mac_wrapper_for "$arch")"
    say "building macOS .app .zip ($arch → ${target:-?})"

    if [[ -z "$target" ]]; then
        RESULT[mac]="fail unknown MAC_ARCH '$arch' (want arm64 or x86_64)"; return
    fi
    if ! rustup target list --installed 2>/dev/null | grep -qx "$target"; then
        RESULT[mac]="skip rust target $target not installed (rustup target add $target)"; return
    fi
    if ! command -v "$wrapper" >/dev/null 2>&1; then
        RESULT[mac]="skip osxcross $wrapper not on PATH (build osxcross, add target/bin to PATH)"; return
    fi
    if ! command -v zip >/dev/null 2>&1; then
        RESULT[mac]="skip zip not found"; return
    fi

    # Point cargo + cc-rs at the osxcross wrappers for this target. cargo reads
    # the per-target linker from CARGO_TARGET_<TRIPLE>_LINKER (triple
    # upper-cased, '-'→'_'); cc-rs (rusqlite-bundled, ring) reads CC/CXX/AR_<triple>.
    local bindir; bindir="$(dirname "$(command -v "$wrapper")")"
    local cxx="${wrapper%clang}clang++"          # oa64-clang → oa64-clang++
    # ar/ranlib are only published under the versioned triple (e.g.
    # aarch64-apple-darwin23-ar); discover it, fall back to llvm-ar.
    local ver_triple ar ranlib
    ver_triple="$(basename "$(ls "$bindir/${target%-darwin}-darwin"*-ar 2>/dev/null | head -1)" 2>/dev/null | sed 's/-ar$//')"
    if [[ -n "$ver_triple" && -x "$bindir/${ver_triple}-ar" ]]; then
        ar="$bindir/${ver_triple}-ar"; ranlib="$bindir/${ver_triple}-ranlib"
    else
        ar="$(command -v llvm-ar || echo ar)"; ranlib="$(command -v llvm-ranlib || echo ranlib)"
    fi
    local cargo_var="CARGO_TARGET_$(echo "$target" | tr 'a-z-' 'A-Z_')_LINKER"

    if ! env \
        "$cargo_var=$wrapper" \
        "CC_${target//-/_}=$wrapper" \
        "CXX_${target//-/_}=$cxx" \
        "AR_${target//-/_}=$ar" \
        "RANLIB_${target//-/_}=$ranlib" \
        cargo build --release --target "$target"; then
        RESULT[mac]="fail cargo build --target $target returned nonzero"; return
    fi
    local bin="target/$target/release/$PKG"
    if [[ ! -f "$bin" ]]; then
        RESULT[mac]="fail expected $bin after build"; return
    fi

    # ── Assemble the .app bundle (just a directory tree + Info.plist). ──
    local app="$DIST/${PKG}.app"
    rm -rf "$app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    install -m755 "$bin" "$app/Contents/MacOS/$PKG"

    cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>            <string>Catacomb</string>
    <key>CFBundleDisplayName</key>     <string>Catacomb</string>
    <key>CFBundleIdentifier</key>      <string>org.codeberg.catacomb</string>
    <key>CFBundleVersion</key>         <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key> <string>${VERSION}</string>
    <key>CFBundleExecutable</key>      <string>${PKG}</string>
    <key>CFBundleIconFile</key>        <string>${PKG}.icns</string>
    <key>CFBundlePackageType</key>     <string>APPL</string>
    <key>LSMinimumSystemVersion</key>  <string>11.0</string>
    <key>NSHighResolutionCapable</key> <true/>
</dict>
</plist>
PLIST

    # Best-effort icon: convert icon.png → .icns if a converter exists; the
    # bundle is valid without it (just falls back to a generic icon).
    if command -v png2icns >/dev/null 2>&1; then
        png2icns "$app/Contents/Resources/$PKG.icns" icon.png >/dev/null 2>&1 \
            || warn "png2icns failed; .app will use a generic icon"
    elif command -v iconutil >/dev/null 2>&1; then
        : # iconutil is macOS-only; on a real Mac you'd build an .iconset here
    else
        warn "no png2icns/iconutil; .app ships without a custom icon"
    fi

    local out="$DIST/${PKG}-${VERSION}-${arch}-macos.zip"
    rm -f "$out"
    # Zip the .app from DIST so the archive root is the bundle. -y keeps any
    # symlinks as links; -r recurses the bundle tree.
    if (cd "$DIST" && zip -q -r -y "$out" "${PKG}.app"); then
        RESULT[mac]="ok $out"
    else
        RESULT[mac]="fail zip returned nonzero"
    fi
    rm -rf "$app"
}

# ── Dispatch ─────────────────────────────────────────────────────────────────
# The Windows/macOS zips don't need the Linux release binary, so skip that
# slow build when a cross target is all that was asked for.
if [[ "$WANT" != "win" && "$WANT" != "mac" ]]; then
    build_binary
fi

case "$WANT" in
    deb)      build_deb ;;
    rpm)      build_rpm ;;
    appimage) build_appimage ;;
    win)      build_win ;;
    mac)      build_mac ;;
    all)      build_deb; build_rpm; build_appimage
              if win_available; then build_win; fi
              if mac_available; then build_mac; fi ;;
    *)        err "unknown target '$WANT' (want: deb|rpm|appimage|win|mac|all)"; exit 2 ;;
esac

# ── Summary ──────────────────────────────────────────────────────────────────
echo
say "package summary"
status=0
for fmt in deb rpm appimage win mac; do
    [[ -v RESULT[$fmt] ]] || continue
    state="${RESULT[$fmt]%% *}"
    detail="${RESULT[$fmt]#* }"
    case "$state" in
        ok)   printf '  \033[1;32m✓\033[0m %-9s %s\n' "$fmt" "$detail" ;;
        skip) printf '  \033[1;33m–\033[0m %-9s %s\n' "$fmt" "$detail" ;;
        fail) printf '  \033[1;31m✗\033[0m %-9s %s\n' "$fmt" "$detail"; status=1 ;;
    esac
done
exit $status
