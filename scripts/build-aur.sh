#!/usr/bin/env bash
# Generate AUR PKGBUILD from template.
set -euo pipefail

BINARY="$1"
VERSION="$2"
DESCRIPTION="SSH connection manager CLI with layered YAML config and Docker bootstrap support"
OWNER=$(gh repo view --json owner -q .owner.login 2>/dev/null || echo "OWNER_PLACEHOLDER")
REPO=$(gh repo view --json name -q .name 2>/dev/null || echo "${BINARY}")

mkdir -p dist

sed \
    -e "s/BINARY_PLACEHOLDER/${BINARY}/g" \
    -e "s/VERSION_PLACEHOLDER/${VERSION}/g" \
    -e "s/OWNER_PLACEHOLDER/${OWNER}/g" \
    -e "s/REPO_PLACEHOLDER/${REPO}/g" \
    -e "s/DESCRIPTION_PLACEHOLDER/${DESCRIPTION}/g" \
    packaging/aur/PKGBUILD.template > dist/PKGBUILD

echo "Built dist/PKGBUILD"
