#!/usr/bin/env bash
set -euo pipefail
staramp_skin_source="${BASH_SOURCE[0]}"
while [[ -L "$staramp_skin_source" ]]; do
    staramp_skin_link_dir="$(cd -P -- "$(dirname -- "$staramp_skin_source")" && pwd)"
    staramp_skin_source="$(readlink "$staramp_skin_source")"
    [[ "$staramp_skin_source" = /* ]] || staramp_skin_source="$staramp_skin_link_dir/$staramp_skin_source"
done
staramp_skin_root="$(cd -P -- "$(dirname -- "$staramp_skin_source")/.." && pwd)"
exec "$staramp_skin_root/target/release/examples/skin-player" "$@"
