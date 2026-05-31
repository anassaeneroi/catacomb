#!/usr/bin/env bash
# Build distributable packages for yt-offline.
#
# Usage:
#   scripts/package.sh [deb|rpm|appimage|all]
#
# With no argument, builds everything that's buildable on the current host.
# Output lands in dist/. Each format is independent — a failure in one
# doesn't abort the others (the script reports a summary at the end).
#
# Prerequisites are installed on demand where possible:
#   - .deb     → cargo-deb           (cargo install cargo-deb)
#   - .rpm     → cargo-generate-rpm  (cargo install cargo-generate-rpm)
#   - AppImage → appimagetool        (downloaded to dist/tools/ if missing)
#
# The release binary is built once up front and reused by every format.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DIST="$ROOT/dist"
TOOLS="$DIST/tools"
mkdir -p "$DIST" "$TOOLS"

# Pull version + name straight from Cargo.toml so packages stay in sync.
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
PKG="yt-offline"
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
    install -Dm644 youtube-backup.desktop "$appdir/usr/share/applications/$PKG.desktop"
    # AppImage wants the desktop file + icon at the AppDir root too.
    cp youtube-backup.desktop "$appdir/$PKG.desktop"
    install -Dm644 icon.png "$appdir/usr/share/icons/hicolor/256x256/apps/$PKG.png"
    cp icon.png "$appdir/$PKG.png"

    # AppRun is the entry point. Exec the bundled binary, forwarding args.
    cat > "$appdir/AppRun" <<'APPRUN'
#!/usr/bin/env bash
HERE="$(dirname "$(readlink -f "${0}")")"
exec "$HERE/usr/bin/yt-offline" "$@"
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

# ── Dispatch ─────────────────────────────────────────────────────────────────
build_binary

case "$WANT" in
    deb)      build_deb ;;
    rpm)      build_rpm ;;
    appimage) build_appimage ;;
    all)      build_deb; build_rpm; build_appimage ;;
    *)        err "unknown target '$WANT' (want: deb|rpm|appimage|all)"; exit 2 ;;
esac

# ── Summary ──────────────────────────────────────────────────────────────────
echo
say "package summary"
status=0
for fmt in deb rpm appimage; do
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
