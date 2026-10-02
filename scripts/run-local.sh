#!/usr/bin/env bash
# Launch this checkout's release build with its optional Nix audio plugins.
set -euo pipefail
script="${BASH_SOURCE[0]}"
while [[ -L "$script" ]]; do
  directory="$(cd -- "$(dirname -- "$script")" && pwd)"
  script="$(readlink "$script")"
  [[ "$script" = /* ]] || script="$directory/$script"
done
root="$(cd -- "$(dirname -- "$script")/.." && pwd)"
plugins="$root/target/alsa-plugins/lib/alsa-lib"
if [[ -z "${ALSA_PLUGIN_DIR:-}" && -d "$plugins" ]]; then
  export ALSA_PLUGIN_DIR="$plugins"
fi
exec "$root/target/release/staramp" "$@"
