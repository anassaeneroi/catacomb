# Maintainer: InannaBeloved <anassaeneroi@pm.me>
pkgname=youtube-backup
pkgver=0.1.0
pkgrel=1
pkgdesc="A small yt-dlp front-end: browse downloaded channels and queue new downloads"
arch=('x86_64' 'aarch64')
url="https://codeberg.org/anassaeneroi/yt-offline"
license=('GPL3')
depends=('yt-dlp' 'mpv' 'sqlite' 'libxcb')
makedepends=('cargo' 'rustup')
source=("git+https://codeberg.org/anassaeneroi/yt-offline.git")
sha256sums=('SKIP')

build() {
    cd "$pkgname"
    cargo build --release --locked
}

package() {
    cd "$pkgname"

    # Install binary
    install -Dm755 target/release/youtube-backup "$pkgdir/usr/bin/youtube-backup"

    # Install desktop file
    install -Dm644 youtube-backup.desktop "$pkgdir/usr/share/applications/youtube-backup.desktop"

    # Install icon (if it exists)
    if [ -f "icon.png" ]; then
        install -Dm644 icon.png "$pkgdir/usr/share/pixmaps/youtube-backup.png"
    fi

    # Install license
    install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"

    # Install config template
    install -Dm644 config.toml "$pkgdir/etc/youtube-backup/config.toml.example"
}
