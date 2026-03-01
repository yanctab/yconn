#!/usr/bin/env bash
# Generate AUR PKGBUILD from template.
#
# TODO: This is a generated template. Review and update before first release:
#   - Verify the sha256sums line — change 'SKIP' to actual checksums for release
#   - Add any optional runtime dependencies (optdepends array)
#   - Add any post-install steps to the package() function if needed
#     (config files, completions, systemd units, etc.)
#   - Test with: makepkg -si in a clean Arch environment
#
set -euo pipefail

BINARY="$1"
VERSION="$2"
OWNER=$(gh repo view --json owner -q .owner.login 2>/dev/null || echo "OWNER_PLACEHOLDER")
REPO=$(gh repo view --json name -q .name 2>/dev/null || echo "${BINARY}")

mkdir -p dist

sed \
    -e "s/BINARY_PLACEHOLDER/${BINARY}/g" \
    -e "s/VERSION_PLACEHOLDER/${VERSION}/g" \
    -e "s/OWNER_PLACEHOLDER/${OWNER}/g" \
    -e "s/REPO_PLACEHOLDER/${REPO}/g" \
    packaging/aur/PKGBUILD.template > dist/PKGBUILD

echo "Built dist/PKGBUILD"
