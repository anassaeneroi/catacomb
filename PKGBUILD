# Maintainer: InannaBeloved <anassaeneroi@pm.me>
pkgname=catacomb
pkgver=0.1.0
pkgrel=1
pkgdesc="Self-hosted archive for YouTube, TikTok, Twitch, Vimeo, Bandcamp, SoundCloud, Odysee and more. Desktop GUI + web UI."
arch=('x86_64' 'aarch64')
url="https://codeberg.org/anassaeneroi/yt-offline"
license=('AGPL-3.0-only')
depends=(
    'yt-dlp'
    'ffmpeg'
    'mpv'
    'sqlite'
    'libxcb'
    'xdg-utils'
)
optdepends=(
    'libnotify: desktop notifications when downloads finish'
)
makedepends=('rust' 'cargo')
# Disable makepkg's environment-level LTO so we don't double up with the
# Cargo.toml profile (which sets lto = "thin"). Full LTO via rust-lld
# previously broke the rusqlite bundled-sqlite link; thin LTO in Cargo
# does the cross-crate inlining we actually want.
options=('!lto')
# Force the checkout dir to $pkgname (catacomb); the Codeberg repo is still
# named yt-offline, so without the `$pkgname::` prefix the clone dir wouldn't
# match the `cd "$pkgname"` below.
source=("$pkgname::git+https://codeberg.org/anassaeneroi/yt-offline.git#branch=main")
sha256sums=('SKIP')

pkgver() {
    cd "$pkgname"
    # 0.1.0.r12.gabcdef0 — last tag, commits since, short hash
    git describe --long --tags --always 2>/dev/null \
        | sed 's/^v//; s/\([^-]*-g\)/r\1/; s/-/./g' \
        || printf "0.1.0.r%s.g%s" \
            "$(git rev-list --count HEAD)" \
            "$(git rev-parse --short HEAD)"
}

build() {
    cd "$pkgname"
    cargo build --release --frozen
}

package() {
    cd "$pkgname"

    install -Dm755 target/release/catacomb "$pkgdir/usr/bin/catacomb"
    install -Dm644 catacomb.desktop "$pkgdir/usr/share/applications/catacomb.desktop"
    install -Dm644 icon.png "$pkgdir/usr/share/pixmaps/catacomb.png"
    install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
    install -Dm644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
}
