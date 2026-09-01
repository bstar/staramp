#!/usr/bin/env bash
# The AppImage. Run inside the same old-glibc container as portable.sh.
#
# This is the answer to the one thing that stops a plain Linux binary working:
# libavcodec's soname differs on every distribution, so the ffmpeg libraries
# travel with the binary and its RUNPATH points at them. ALSA deliberately does
# not travel: it dlopens the host's own plugin to reach PipeWire or PulseAudio,
# and a bundled copy would find nothing to play through. Every desktop Linux
# already has it.
#
# Assembled by hand rather than with linuxdeploy, because linuxdeploy ships as
# an AppImage and an AppImage cannot be executed inside a container without
# FUSE. Doing it directly is a dozen lines, needs nothing that has to run, and
# leaves the exclude list somewhere it can be read.
set -euo pipefail
cd "$(dirname "$0")/../.."

. scripts/dist/deps-debian.sh
. "$HOME/.cargo/env"
apt-get install -y -qq --no-install-recommends patchelf squashfs-tools

ver=$(sed -n '0,/^version = /s/^version = "\(.*\)"/\1/p' Cargo.toml)
out=${DIST_DIR:-dist}
mkdir -p "$out"

cargo build --release --locked
# Not `target/`: the container is handed its own CARGO_TARGET_DIR so it cannot
# leave a Debian binary where the host's next `cargo run` expects a native one.
bin="${CARGO_TARGET_DIR:-target}/release/staramp"
scripts/dist/glibc-floor.sh "$bin"

work=$(mktemp -d)
appdir=$work/AppDir
install -Dm755 "$bin"                        "$appdir/usr/bin/staramp"
install -Dm644 packaging/staramp.desktop     "$appdir/staramp.desktop"
install -Dm644 packaging/staramp.png         "$appdir/staramp.png"
install -Dm644 packaging/staramp.desktop     "$appdir/usr/share/applications/staramp.desktop"
install -Dm644 packaging/staramp.png         "$appdir/usr/share/icons/hicolor/256x256/apps/staramp.png"
install -Dm644 packaging/staramp.svg         "$appdir/usr/share/icons/hicolor/scalable/apps/staramp.svg"
install -Dm644 README.md LICENSE NOTICE -t   "$appdir/usr/share/doc/staramp/"
cp "$appdir/staramp.png" "$appdir/.DirIcon"
mkdir -p "$appdir/usr/lib"

# What stays behind, and why. This list is deliberately short: everything else
# travels, including things an AppImage excludelist would normally drop.
#
#   libasound        must be the host's. It dlopens the host's plugin modules
#                    to reach PipeWire or PulseAudio; a bundled copy finds
#                    nothing to play through, which is silence on every modern
#                    desktop.
#   the C runtime    bundling a C library into an AppImage is how they break.
#   libgcc_s,        the host's is never older than bullseye's, and a bundled
#   libstdc++        old one is a real hazard on a newer host.
#
# libX11 and friends used to be on this list, on the reasoning that any desktop
# has them. They are pulled in transitively by libpulse under libavcodec, and
# a headless server or a minimal container has none of them: the player then
# refuses to start over ssh, which is where a terminal music player most wants
# to work. Nothing here integrates with a display, so carrying our own costs
# only size.
keep_out='^(ld-linux|libc\.so|libm\.so|libdl\.so|libpthread\.so|librt\.so|libresolv\.so|libutil\.so|libnsl\.so|libgcc_s\.so|libstdc\+\+\.so|libasound\.so)'

# Walk NEEDED transitively. ldd on the binary already reports the whole graph,
# so one pass is enough; the loop is over what it found, not over levels.
ldd "$appdir/usr/bin/staramp" | awk '{print $3}' | grep -E '^/' | sort -u | while read -r lib; do
  base=$(basename "$lib")
  if echo "$base" | grep -qE "$keep_out"; then
    echo "host:   $base"
    continue
  fi
  echo "bundle: $base"
  cp -L "$lib" "$appdir/usr/lib/"
done

# The loader resolves the bundled copies with no environment set at all, and
# RUNPATH beats ld.so.cache, so a host libavcodec cannot shadow ours even when
# the soname matches exactly.
patchelf --set-rpath '$ORIGIN/../lib' "$appdir/usr/bin/staramp"
for so in "$appdir"/usr/lib/*.so*; do
  patchelf --set-rpath '$ORIGIN' "$so"
done

cat > "$appdir/AppRun" <<'EOF'
#!/bin/sh
# The binary's RUNPATH is $ORIGIN/../lib, so no LD_LIBRARY_PATH is needed and
# none is exported: nothing this launches should inherit our library path.
#
# "$@" is not optional. staramp takes a playlist or a directory, and a dozen
# subcommands (ctl, probe, decode, scan, query). An AppImage that swallows
# argv would be useless for all of them.
HERE=$(dirname "$(readlink -f "$0")")
exec "$HERE/usr/bin/staramp" "$@"
EOF
chmod +x "$appdir/AppRun"

# The type-2 runtime, downloaded rather than executed: this is the small ELF
# that gets prepended to the filesystem image and does the mounting at run
# time. Nothing here has to run it, which is the point.
curl -fsSL -o "$work/runtime" \
  https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64
chmod +x "$work/runtime"

# gzip rather than zstd: every AppImage runtime in the wild can read it, and
# this file is meant for the machines we have not thought of.
mksquashfs "$appdir" "$work/fs.squashfs" -root-owned -noappend -comp gzip -no-progress

target="$out/staramp-$ver-x86_64.AppImage"
cat "$work/runtime" "$work/fs.squashfs" > "$target"
chmod +x "$target"
ls -la "$target"
echo "wrote $target"
