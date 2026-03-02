#!/usr/bin/env bash
# Build an Arch Linux .pkg.tar.zst package from the musl binary.
set -euo pipefail

BINARY="$1"
VERSION="$2"
TARGET="x86_64-unknown-linux-musl"
PKGREL=1
DESCRIPTION="SSH connection manager CLI with layered YAML config and Docker bootstrap support"
URL="https://github.com/yanctab/yconn"

STAGING="dist/.pkg-staging"
rm -rf "${STAGING}"
mkdir -p "${STAGING}/usr/bin"
mkdir -p "${STAGING}/usr/share/man/man1"

cp "target/${TARGET}/release/${BINARY}" "${STAGING}/usr/bin/${BINARY}"

if [[ -f "docs/man/${BINARY}.1" ]]; then
    gzip -c "docs/man/${BINARY}.1" > "${STAGING}/usr/share/man/man1/${BINARY}.1.gz"
fi

ISIZE=$(find "${STAGING}" -type f -exec wc -c {} + | tail -1 | awk '{print $1}')

cat > "${STAGING}/.PKGINFO" << EOF
pkgname = ${BINARY}
pkgver = ${VERSION}-${PKGREL}
pkgdesc = ${DESCRIPTION}
url = ${URL}
builddate = $(date +%s)
packager = Unknown Packager
size = ${ISIZE}
arch = x86_64
license = MIT
depend = openssh
EOF

OUTFILE="dist/${BINARY}-${VERSION}-${PKGREL}-x86_64.pkg.tar.zst"
tar -C "${STAGING}" -cf - . | zstd -q -o "${OUTFILE}"

rm -rf "${STAGING}"
echo "Built ${OUTFILE}"
