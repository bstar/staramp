#!/usr/bin/env bash
# Build prerequisites inside a Debian or Ubuntu container.
#
# Rust comes from rustup rather than apt: the MSRV is newer than anything
# bookworm or bullseye package, and the whole point of building in an old
# container is the old *glibc*, not an old toolchain.
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive
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
