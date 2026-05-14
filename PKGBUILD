# Maintainer: InannaBeloved <anassaeneroi@pm.me>
pkgname=yt-offline
pkgver=0.1.0
pkgrel=1
pkgdesc="A desktop app for archiving YouTube channels with yt-dlp"
arch=('x86_64' 'aarch64')
url="https://codeberg.org/anassaeneroi/yt-offline"
license=('AGPL-3.0-only')
depends=('yt-dlp' 'mpv' 'sqlite' 'libxcb')
makedepends=('rust' 'cargo')
options=('!lto')  # rusqlite bundled sqlite cannot be LTO-linked with rust-lld
source=("git+https://codeberg.org/anassaeneroi/yt-offline.git")
sha256sums=('SKIP')

build() {
    cd "$pkgname"
    cargo build --release --locked
}

package() {
    cd "$pkgname"

    install -Dm755 target/release/yt-offline "$pkgdir/usr/bin/yt-offline"
    install -Dm644 youtube-backup.desktop "$pkgdir/usr/share/applications/yt-offline.desktop"

    if [ -f "icon.png" ]; then
        install -Dm644 icon.png "$pkgdir/usr/share/pixmaps/yt-offline.png"
    fi

    install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
    install -Dm644 config.toml "$pkgdir/etc/yt-offline/config.toml.example"
}
