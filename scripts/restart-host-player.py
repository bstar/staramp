#!/usr/bin/env python3
"""Restart this Linux checkout's host player after a successful build."""
import argparse
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, required=True)
    parser.add_argument('--session', default='player')
    args = parser.parse_args()
    repo = args.repo.resolve()
    executable = repo / 'target/release/staramp'
    launcher = repo / 'scripts/run-local.sh'
    subprocess.run([str(launcher), '--version'], check=True)
    runtime = Path(os.environ.get('XDG_RUNTIME_DIR', str(Path.home() / '.local/staramp/cache/run')))
    if 'XDG_RUNTIME_DIR' in os.environ:
        runtime /= 'staramp'
    players = []
    sessions = {args.session}
    for process in Path('/proc').iterdir():
        if not process.name.isdigit():
            continue
        try:
            if process.stat().st_uid != os.getuid():
                continue
            target = os.readlink(process / 'exe').removesuffix(' (deleted)')
            if target != str(executable):
                continue
            argv = (process / 'cmdline').read_bytes().decode().strip('\0').split('\0')[1:]
            if '--graphical-server' in argv:
                sessions.add(argv[argv.index('--graphical-server') + 1])
            elif 'graphical' not in argv or '--ssh' in argv:
                continue
            players.append(int(process.name))
        except (OSError, IndexError):
            continue
    for name in sessions:
        if not name or any(c not in 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-' for c in name):
            raise RuntimeError(f'Invalid session name: {name!r}')
    # Keep a private recovery snapshot before replacing a playback owner.
    if players:
        snapshot = {}
        for request in ('mirror', 'view', 'queue'):
            result = subprocess.run([str(launcher), 'ctl', request], capture_output=True, text=True)
            if result.returncode == 0:
                try:
                    snapshot[request] = json.loads(result.stdout)
                except ValueError:
                    pass
        fd, backup = tempfile.mkstemp(prefix='staramp-before-restart-', suffix='.json')
        with os.fdopen(fd, 'w') as output:
            json.dump(snapshot, output)
        print(f'Playback recovery snapshot: {backup}', flush=True)
    for pid in players:
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        alive = []
        for pid in players:
            try:
                if Path(f'/proc/{pid}/stat').read_text().split(') ', 1)[1][0] != 'Z':
                    alive.append(pid)
            except FileNotFoundError:
                pass
        if not alive:
            break
        time.sleep(0.1)
    else:
        raise RuntimeError(f'Players did not stop after SIGTERM: {alive}')
    for name in sorted(sessions):
        fd, log = tempfile.mkstemp(prefix=f'staramp-{name}-restart-', suffix='.log')
        with os.fdopen(fd, 'wb') as output:
            child = subprocess.Popen([str(launcher), '--graphical-server', name],
                                     cwd=repo, stdin=subprocess.DEVNULL,
                                     stdout=output, stderr=output, start_new_session=True)
        path = runtime / 'graphical' / f'{name}.sock'
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if child.poll() is not None:
                raise RuntimeError(f'Player exited with {child.returncode}; see {log}')
            try:
                running = os.stat(f'/proc/{child.pid}/exe')
                built = executable.stat()
                if (running.st_dev, running.st_ino) != (built.st_dev, built.st_ino):
                    time.sleep(0.1)
                    continue
                with socket.socket(socket.AF_UNIX) as connection:
                    connection.connect(str(path))
                break
            except OSError:
                time.sleep(0.1)
        else:
            raise RuntimeError(f'Player did not become ready; see {log}')
        running = os.stat(f'/proc/{child.pid}/exe')
        built = executable.stat()
        if (running.st_dev, running.st_ino) != (built.st_dev, built.st_ino):
            raise RuntimeError('Restarted player does not use the current release executable')
        print(f'Restarted host session {name}: PID {child.pid}, current release executable', flush=True)
    subprocess.run([str(launcher), 'ctl', 'status'], check=True)
    print('Host ready. The Mac frontend can reconnect; the saved session can be resumed.')


if __name__ == '__main__':
    main()
