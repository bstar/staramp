#!/usr/bin/env bash
# Native Apple Silicon release package. Run on a macOS builder.
set -euo pipefail
cd "$(dirname "$0")/../.."
[ "$(uname -s)" = Darwin ] || { echo "macOS build requires a Mac" >&2; exit 1; }
ver=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
out=${DIST_DIR:-dist}
mkdir -p "$out"
cargo build --release --locked --target aarch64-apple-darwin
bin="${CARGO_TARGET_DIR:-target}/aarch64-apple-darwin/release/staramp"
"$bin" --version
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
stage="$work/staramp-$ver"
mkdir -p "$stage"
install -m755 "$bin" "$stage/staramp"
cp README.md LICENSE "$stage/"
[ ! -f NOTICE ] || cp NOTICE "$stage/"
[ ! -d LICENSES ] || cp -R LICENSES "$stage/"
printf '%s\n' 'Requires Homebrew FFmpeg: brew install ffmpeg' > "$stage/INSTALL.txt"
tar -C "$work" -czf "$out/staramp-$ver-aarch64-apple-darwin.tar.gz" "staramp-$ver"
echo "wrote $out/staramp-$ver-aarch64-apple-darwin.tar.gz"
