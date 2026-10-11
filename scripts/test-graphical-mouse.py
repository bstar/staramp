#!/usr/bin/env python3
"""Check zero mouse coordinates and the main close X through a private PTY.

Run after a Nix build, passing the executable to test:
    nix develop -c python3 scripts/test-graphical-mouse.py target/release/staramp
"""
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

binary = str(Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='staramp-mouse-') as temporary:
    root = Path(temporary)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 100, 1000, 800))
    env = os.environ.copy()
    env.update(TERM='xterm-kitty', HOME=str(root), STARAMP_DIR=str(root / 'data'),
               STARAMP_CONFIG_DIR=str(root / 'config'), XDG_RUNTIME_DIR=str(root / 'runtime'),
               STAR_GRAPHICS_SYSTEM_FONTS='0', DBUS_SESSION_BUS_ADDRESS='unix:path=/nonexistent/staramp-test-bus')
    for key in ['TMUX', 'STY', 'SSH_CONNECTION', 'SSH_TTY', 'KITTY_WINDOW_ID', 'KITTY_LISTEN_ON']:
        env.pop(key, None)
    # Initialize the disposable history schema before its UI/worker readers
    # start concurrently; this check targets terminal input and shutdown.
    subprocess.run([binary, 'scrobble', 'status'], env=env, stdin=subprocess.DEVNULL,
                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True)

    def session():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    child = subprocess.Popen([binary, 'graphical'], stdin=slave, stdout=slave, stderr=slave,
                             env=env, preexec_fn=session)
    os.close(slave)
    output, pending = bytearray(), bytearray()

    def pump(duration):
        deadline = time.monotonic() + duration
        while time.monotonic() < deadline:
            if not select.select([master], [], [], min(.05, max(0, deadline - time.monotonic())))[0]:
                continue
            try:
                chunk = os.read(master, 65536)
            except OSError:
                return
            if not chunk:
                return
            output.extend(chunk)
            pending.extend(chunk)
            for match in list(re.finditer(rb'\x1b_G([^\x1b]*)\x1b\\', pending)):
                if b'a=q' in match.group(1):
                    ident = re.search(rb'(?:^|,)i=(\d+)', match.group(1))
                    if ident:
                        os.write(master, b'\x1b_Gi=' + ident.group(1) + b';OK\x1b\\')
                pending[:] = pending.replace(match.group(0), b'')
            for query, reply in [(b'\x1b[16t', b'\x1b[6;20;10t'), (b'\x1b[14t', b'\x1b[4;800;1000t'),
                                 (b'\x1b[c', b'\x1b[?62;4;22c'), (b'\x1b[5n', b'\x1b[0n'),
                                 (b'\x1b[?u', b'\x1b[?0u')]:
                if query in pending:
                    os.write(master, reply)
                    pending[:] = pending.replace(query, b'')
            if len(pending) > 4096:
                del pending[:-512]

    def diagnostic():
        at = output.find(b'panicked at')
        return bytes(output[max(0, at - 120):at + 1600] if at >= 0 else output[:2000] + output[-1000:])

    try:
        deadline = time.monotonic() + 20
        while b'\x1b[?1016h' not in output:
            pump(.1)
            assert child.poll() is None, diagnostic()
            assert time.monotonic() < deadline, 'Pixel mouse mode was not enabled'
        # Origin and edge reports must survive the input parser, including
        # reports received while the graphical frontend is still starting.
        for packet in [b'\x1b[<35;0;0M', b'\x1b[<35;0;200M', b'\x1b[<35;200;0M']:
            os.write(master, packet)
            pump(.2)
            assert child.poll() is None, diagnostic()
        pump(2)
        # Main header close target: x=960..989, y=7..30 at this viewport.
        os.write(master, b'\x1b[<0;976;20M\x1b[<0;976;20m')
        deadline = time.monotonic() + 8
        while child.poll() is None and time.monotonic() < deadline:
            pump(.1)
        assert child.poll() == 0, diagnostic()
        pump(.1)
        assert b'\x1b[?1016l' in output, 'Pixel mouse mode was not restored'
        assert b'panicked at' not in output, bytes(output[-2000:])
        print(json.dumps({'real_frontend_pty': True, 'zero_coordinates_survived': True,
                          'main_close_exits_frontend': True, 'pixel_mode_restored': True}))
    finally:
        # Every child belongs to this private process group, never a user session.
        try:
            os.killpg(child.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        child.wait(timeout=5)
        os.close(master)
