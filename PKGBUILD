pkgname=kcli
pkgver=0.9
pkgrel=1
pkgdesc="CapyCachy Kernel Manager"
arch=('x86_64')
url="https://github.com/lseman/kcli"
license=('MIT') # Change this if your project uses a different license
depends=('rust' 'cargo' 'git')
source=("git+https://github.com/lseman/kcli.git")
sha256sums=('SKIP')

pkgver() {
  cd "$srcdir/kcli"
  printf "r%s.%s" "$(git rev-list --count HEAD)" "$(git rev-parse --short HEAD)"
}

build() {
  cd "$srcdir/kcli"
  cargo build --release
}

package() {
  cd "$srcdir/kcli"
  install -Dm755 "target/release/kcli" "$pkgdir/usr/bin/kcli"
}
