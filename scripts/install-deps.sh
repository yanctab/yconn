#!/usr/bin/env bash
# Install required system packages for building yconn, with distro detection.
set -euo pipefail

if [[ ! -f /etc/os-release ]]; then
    echo "Cannot detect distribution: /etc/os-release not found."
    echo "Please install manually: musl-tools (or musl), pandoc, zstd"
    exit 1
fi

# shellcheck source=/dev/null
source /etc/os-release

ID="${ID:-}"
ID_LIKE="${ID_LIKE:-}"

is_debian_like() {
    [[ "$ID" == "debian" || "$ID" == "ubuntu" ]] && return 0
    [[ "$ID_LIKE" == *"debian"* || "$ID_LIKE" == *"ubuntu"* ]] && return 0
    return 1
}

is_arch_like() {
    [[ "$ID" == "arch" ]] && return 0
    [[ "$ID_LIKE" == *"arch"* ]] && return 0
    return 1
}

if is_debian_like; then
    echo "Detected Debian/Ubuntu-based distribution: $ID"
    sudo apt-get install -y musl-tools pandoc zstd
elif is_arch_like; then
    echo "Detected Arch Linux-based distribution: $ID"
    sudo pacman -S --noconfirm musl pandoc zstd
else
    echo "Unsupported distribution: $ID. Please install manually: musl-tools (or musl), pandoc, zstd"
    exit 1
fi
