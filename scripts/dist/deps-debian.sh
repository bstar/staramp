#!/usr/bin/env bash
# Build prerequisites inside a Debian or Ubuntu container.
#
# Rust comes from rustup rather than apt: the MSRV is newer than anything
# bookworm or bullseye package, and the whole point of building in an old
# container is the old *glibc*, not an old toolchain.
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive
# Bullseye left LTS in August 2026: its packages moved to archive.debian.org,
# the security suite is not archived at all, and the Release file there is no
# longer re-signed. What this container is for is its glibc, not its security
# updates, so point apt at the archive's main suite alone and let it accept an
# expired file. A supported release is left alone.
if grep -q '^VERSION_CODENAME=bullseye' /etc/os-release; then
  echo 'deb http://archive.debian.org/debian bullseye main' > /etc/apt/sources.list
  echo 'Acquire::Check-Valid-Until "false";' > /etc/apt/apt.conf.d/99bullseye-eol
fi
apt-get update -qq
apt-get install -y -qq --no-install-recommends \
  ca-certificates curl file git xz-utils \
  build-essential pkg-config clang libclang-dev \
  libasound2-dev libdbus-1-dev \
  libavcodec-dev libavformat-dev libavutil-dev libswresample-dev

if ! command -v cargo >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path
fi
. "$HOME/.cargo/env"

# bindgen needs libclang, and its path differs per release.
LIBCLANG_PATH=$(dirname "$(find /usr/lib -name 'libclang.so*' -o -name 'libclang-*.so*' 2>/dev/null | head -1)")
export LIBCLANG_PATH
echo "LIBCLANG_PATH=$LIBCLANG_PATH"
