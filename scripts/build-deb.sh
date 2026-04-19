#!/usr/bin/env bash
# Build a .deb package from the musl binary.
set -euo pipefail

BINARY="$1"
VERSION="$2"
TARGET="x86_64-unknown-linux-musl"
PKG="${BINARY}_${VERSION}_amd64"

mkdir -p "dist/${PKG}/DEBIAN"
mkdir -p "dist/${PKG}/usr/bin"
mkdir -p "dist/${PKG}/usr/share/man/man1"
mkdir -p "dist/${PKG}/usr/share/doc/${BINARY}"

cp "target/${TARGET}/release/${BINARY}" "dist/${PKG}/usr/bin/${BINARY}"

if [[ -f "docs/man/${BINARY}.1" ]]; then
    gzip -c "docs/man/${BINARY}.1" > "dist/${PKG}/usr/share/man/man1/${BINARY}.1.gz"
fi

sed \
    -e "s/VERSION_PLACEHOLDER/${VERSION}/g" \
    -e "s/BINARY_PLACEHOLDER/${BINARY}/g" \
    packaging/deb/control > "dist/${PKG}/DEBIAN/control"

dpkg-deb --build "dist/${PKG}" "dist/${PKG}.deb"
echo "Built dist/${PKG}.deb"
