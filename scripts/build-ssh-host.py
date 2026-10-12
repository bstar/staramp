#!/usr/bin/env python3
"""Pull and build this published revision on a remote music server."""
import argparse
import base64
from pathlib import Path
import shlex
import subprocess

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('host', help='SSH destination, e.g. bstar@192.168.86.57')
    parser.add_argument('--repo', required=True, help='Absolute remote STAR/AMP checkout path')
    parser.add_argument('--control-path', help='Reuse an authenticated SSH multiplexing socket')
    parser.add_argument('--no-restart', action='store_true', help='Build without restarting the host player')
    parser.add_argument('--session', default='player', help='Host session to start after building')
    parser.add_argument('--cleanup-script', type=Path,
                        default=Path.home() / '.codex/skills/rust-build-hygiene/scripts/clean_build_cache.py')
    args = parser.parse_args()
    if not args.repo.startswith('/'):
        parser.error('--repo must be an absolute path')
    cleanup = base64.b64encode(args.cleanup_script.read_bytes()).decode('ascii')
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    ssh = ['ssh', '-o', 'BatchMode=yes', '-o', 'ServerAliveInterval=15',
           '-o', 'ServerAliveCountMax=3']
    if args.control_path:
        ssh += ['-S', args.control_path]
    script = r'''set -euo pipefail
repo=$1
expected=$2
cleanup_file=$(mktemp /tmp/staramp-build-cleanup.XXXXXXXX.py)
python3 -c 'import base64,sys; open(sys.argv[1], "wb").write(base64.b64decode(sys.argv[2]))' "$cleanup_file" "$3"
cleanup() {
    result=$?
    trap - EXIT
    if ! python3 "$cleanup_file" --apply --projects "$repo" --no-xcode; then
        echo 'Compiler cleanup failed; see output above.' >&2
        result=1
    fi
    rm -f -- "$cleanup_file"
    exit "$result"
}
trap 'rm -f -- "$cleanup_file"' EXIT
cd -- "$repo"
if [[ -n "$(git status --porcelain)" ]]; then
    echo 'Remote checkout has changes; preserve or commit them before updating.' >&2
    git status --short
    exit 1
fi
if [[ -f .cargo/config.toml ]] && rg -q '^\[patch\.' .cargo/config.toml; then
    echo 'Remove the remote development dependency override before building a release.' >&2
    exit 1
fi
[[ "$(git branch --show-current)" = main ]] || { echo 'Remote checkout must be on main.' >&2; exit 1; }
git_options=(-c 'url.https://github.com/.insteadOf=git@github.com:')
if command -v gh >/dev/null && gh auth status >/dev/null 2>&1; then
    git_options+=(-c credential.helper= -c 'credential.helper=!gh auth git-credential')
fi
git "${git_options[@]}" fetch origin main
actual=$(git rev-parse origin/main)
[[ "$actual" = "$expected" ]] || { echo "GitHub revision $actual differs from local $expected; publish or update locally first." >&2; exit 1; }
git merge --ff-only origin/main
[[ "$(git rev-parse HEAD)" = "$expected" ]] || { echo 'Remote branch is ahead of the requested revision.' >&2; exit 1; }
export CARGO_NET_GIT_FETCH_WITH_CLI=true
export CARGO_PROFILE_DEV_DEBUG=1 CARGO_PROFILE_TEST_DEBUG=1
export CARGO_PROFILE_DEV_INCREMENTAL=false CARGO_PROFILE_TEST_INCREMENTAL=false
export GIT_CONFIG_COUNT=1
export GIT_CONFIG_KEY_0=url.https://github.com/.insteadOf
export GIT_CONFIG_VALUE_0=git@github.com:
trap cleanup EXIT
nix --extra-experimental-features 'nix-command flakes' develop -c cargo build --release --locked
./scripts/run-local.sh --version
printf 'Built remote STAR/AMP revision %s\n' "$expected"
if [[ "$4" = restart ]]; then
    python3 scripts/restart-host-player.py --repo "$repo" --session "$5"
fi
'''
    command = 'bash -s -- ' + shlex.join([args.repo, revision, cleanup, "skip" if args.no_restart else "restart", args.session])
    subprocess.run(ssh + [args.host, command], input=script, text=True, check=True)


if __name__ == '__main__':
    main()
