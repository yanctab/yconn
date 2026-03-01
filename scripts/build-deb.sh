#!/usr/bin/env bash
# Build a .deb package from the musl binary.
#
# TODO: This is a generated template. Review and update before first release:
#   - Add any runtime dependencies to packaging/deb/control (Depends: field)
#   - Add any config files, systemd units, or shell completions that should
#     be packaged (copy them into the staging directory below)
#   - Test with: dpkg-deb --build dist/<pkg> and install on a clean system
#
set -euo pipefail

BINARY="$1"
VERSION="$2"
TARGET="x86_64-unknown-linux-musl"
PKG="${BINARY}_${VERSION}_amd64"

mkdir -p "dist/${PKG}/DEBIAN"
mkdir -p "dist/${PKG}/usr/bin"
mkdir -p "dist/${PKG}/usr/share/man/man1"

cp "target/${TARGET}/release/${BINARY}" "dist/${PKG}/usr/bin/${BINARY}"

# TODO: copy any additional files here, for example:
# cp packaging/deb/completions/${BINARY}.bash "dist/${PKG}/usr/share/bash-completion/completions/${BINARY}"
# cp packaging/deb/${BINARY}.service "dist/${PKG}/lib/systemd/system/${BINARY}.service"

if [[ -f "docs/man/${BINARY}.1" ]]; then
    gzip -c "docs/man/${BINARY}.1" > "dist/${PKG}/usr/share/man/man1/${BINARY}.1.gz"
fi

sed \
    -e "s/VERSION_PLACEHOLDER/${VERSION}/g" \
    -e "s/BINARY_PLACEHOLDER/${BINARY}/g" \
    packaging/deb/control > "dist/${PKG}/DEBIAN/control"

dpkg-deb --build "dist/${PKG}" "dist/${PKG}.deb"
echo "Built dist/${PKG}.deb"
