#!/usr/bin/env python3
"""Check mouse input, optional resize redraws, and closing through a private PTY.

Run after a Nix build, passing the executable to test:
    nix develop -c python3 scripts/test-graphical-mouse.py target/release/staramp --resize
"""
import argparse
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
import tempfile
import termios
import threading
import time
import wave

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary')
parser.add_argument('--resize', action='store_true', help='also exercise resize redraws and density changes')
parser.add_argument('--local-hover', action='store_true', help='check queue hover without controller input')
parser.add_argument('--motion-burst', action='store_true', help='check rapid hover input before closing')
parser.add_argument('--fold-playlist', action='store_true', help='collapse and expand the playlist repeatedly at both densities')
parser.add_argument('--imports', action='store_true', help='launch the graphical import reviewer with a disposable album')
args = parser.parse_args()
binary = str(Path(args.binary).resolve())
with tempfile.TemporaryDirectory(prefix='staramp-mouse-') as temporary:
    root = Path(temporary)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 100, 1000, 800))
    env = os.environ.copy()
    env.update(TERM='xterm-kitty', HOME=str(root), STARAMP_DIR=str(root / 'data'),
               STARAMP_CONFIG_DIR=str(root / 'config'), XDG_RUNTIME_DIR=str(root / 'runtime'),
               STAR_GRAPHICS_SYSTEM_FONTS='0', DBUS_SESSION_BUS_ADDRESS='unix:path=/nonexistent/staramp-test-bus')
    if args.local_hover or args.fold_playlist:
        env['STARAMP_LOG'] = 'staramp=debug,starkit=debug'
    for key in ['TMUX', 'STY', 'SSH_CONNECTION', 'SSH_TTY', 'KITTY_WINDOW_ID', 'KITTY_LISTEN_ON']:
        env.pop(key, None)
    # Initialize the disposable history schema before its UI/worker readers
    # start concurrently; this check targets terminal input and shutdown.
    subprocess.run([binary, 'scrobble', 'status'], env=env, stdin=subprocess.DEVNULL,
                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True)
    launch = [binary, 'graphical']
    if args.imports:
        library = root / 'library'
        library.mkdir()
        staging = root / 'inbox with spaces'
        album = staging / 'Fixture Artist' / 'Fixture Album'
        album.mkdir(parents=True)
        track = album / '01 - Fixture.wav'
        with wave.open(str(track), 'wb') as audio:
            audio.setparams((1, 2, 44100, 0, 'NONE', 'not compressed'))
            audio.writeframes(b'\0\0' * 4410)
        (root / 'config/config.toml').write_text(f'library_root = "{library}"\n')
        launch = [binary, 'import', '--graphical', str(staging)]

    def session():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    child = subprocess.Popen(launch, stdin=slave, stdout=slave, stderr=slave,
                             env=env, preexec_fn=session)
    os.close(slave)
    output, pending = bytearray(), bytearray()
    geometry = [40, 100, 1000, 800]

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
            rows, columns, width, height = geometry
            for query, reply in [(b'\x1b[16t', f'\x1b[6;{height // rows};{width // columns}t'.encode()),
                                 (b'\x1b[14t', f'\x1b[4;{height};{width}t'.encode()),
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
        if args.resize:
            deadline = time.monotonic() + 20
            while b'\x1b_Ga=T,' not in output:
                pump(.1)
                assert child.poll() is None, diagnostic()
                assert time.monotonic() < deadline, diagnostic()
            pump(.5)
            # Include an unchanged final size: SIGWINCH can coalesce a rapid
            # shrink/grow round trip before either intermediate frame is drawn.
            for rows, columns, width, height in [(40, 100, 1000, 800), (40, 100, 1001, 801),
                                                  (50, 140, 1400, 1000),
                                                  (30, 80, 800, 600), (30, 84, 1680, 1200),
                                                  (17, 70, 700, 340), (40, 100, 1000, 800)]:
                geometry[:] = rows, columns, width, height
                start = len(output)
                fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack('HHHH', *geometry))
                os.kill(child.pid, signal.SIGWINCH)
                pump(2)
                assert child.poll() is None, diagnostic()
                wire = bytes(output[start:])
                assert b'\x1b[2J' in wire, f'Missing full redraw after resize to {geometry}: {wire[:500]!r}'
                regions = re.findall(rb'\x1b_Ga=T,([^;]*);', wire)
                covered = 0
                for region in regions:
                    fields = dict(item.split(b'=', 1) for item in region.split(b',') if b'=' in item)
                    covered += int(fields[b'c']) * int(fields[b'r'])
                assert covered >= rows * columns, f'Incomplete image replacement after resize to {geometry}: {covered} cells'
            # A rapid shrink/grow may be coalesced into a single SIGWINCH;
            # the final presentation must cover the settled window completely.
            start = len(output)
            for shape in [(20, 75, 750, 400), (42, 130, 1300, 840), (37, 96, 1152, 814)]:
                geometry[:] = shape
                fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack('HHHH', *geometry))
            pump(3)
            assert child.poll() is None, diagnostic()
            wire = bytes(output[start:])
            assert b'\x1b[2J' in wire, 'No redraw at the settled size after rapid resizing'
            assert b'\x1b_Ga=T,' in wire, 'No image replacement after rapid resizing'
            covered = 0
            for region in re.findall(rb'\x1b_Ga=T,([^;]*);', wire):
                fields = dict(item.split(b'=', 1) for item in region.split(b',') if b'=' in item)
                covered += int(fields[b'c']) * int(fields[b'r'])
            assert covered >= geometry[0] * geometry[1], 'Settled window was only partially replaced'
            geometry[:] = 40, 100, 1000, 800
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack('HHHH', *geometry))
            pump(2)
            print(json.dumps({'resize_redraws': 9, 'rapid_drag_settled': True, 'coalesced_round_trip': True,
                              'grow_shrink': True, 'density_changes': True,
                              'native_fallback_round_trip': True}))
        if args.local_hover or args.fold_playlist:
            def ctl(*request):
                result = subprocess.run([binary, 'ctl', *request], env=env, stdin=subprocess.DEVNULL,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True, text=True)
                return result.stdout.strip()
            # These private queue entries are never played or opened.
            assert ctl('set-queue', json.dumps([f'fixture/track-{i}.flac' for i in range(20)])) == '20'
            view = json.loads(ctl('view'))
            view.update(show_equalizer=False, show_album=False, show_scrobbler=False, show_playlist=True)
            assert ctl('set-view', json.dumps(view)) == 'ok'
            # The empty startup queue opens a picker; dismiss it before
            # exercising the newly populated private queue underneath.
            os.write(master, b'\x1b')
            pump(1)
        if args.fold_playlist:
            # AMP reserves a first press in an unfocused module for focus.
            # Focus a body row before counting header actions.
            os.write(master,b'\x1b[<0;600;330M\x1b[<0;600;330m')
            pump(.4)
            view=json.loads(ctl('view'))
            log = next(root.rglob('staramp.log'))
            folds = lambda: len(re.findall(r'action="?fold:playlist"?',log.read_text()))
            before = folds()
            for density in [1, 2]:
                geometry[:] = 40, 100, 1000 * density, 800 * density
                fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack('HHHH', *geometry))
                os.kill(child.pid, signal.SIGWINCH)
                pump(1)
                for _ in range(6):
                    x, y = 40 * density + 1, 254 * density + 1
                    os.write(master, f'\x1b[<0;{x};{y}M\x1b[<0;{x};{y}m'.encode())
                    pump(.3)
                    assert child.poll() is None, diagnostic()
            if folds() != before + 12:
                Path('/tmp/staramp-collapse-frontend-last.log').write_text(log.read_text())
            assert folds() == before + 12, ('Playlist header stopped responding after collapse', before, folds(), [line for line in log.read_text().splitlines() if 'control activated' in line][-15:])
            assert json.loads(ctl('view'))['show_playlist'], 'Folding closed the playlist'
            geometry[:] = 40, 100, 1000, 800
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack('HHHH', *geometry))
            os.kill(child.pid, signal.SIGWINCH)
            pump(1)
            print(json.dumps({'playlist_collapse_expand':12,'density_1_and_2':True,'surface_metadata_valid':True}))
        if args.local_hover:
            os.write(master, b'\x1b[<35;600;330M')
            pump(.4)
            log = next(root.rglob('staramp.log'))
            admitted = log.read_text().count('Graphical input admitted')
            start = len(output)
            started = time.monotonic()
            os.write(master, b'\x1b[<35;600;352M')
            while b'\x1b_Ga=T,' not in output[start:]:
                pump(.005)
                assert child.poll() is None, diagnostic()
                assert time.monotonic() - started < 1, 'Local hover did not repaint'
            latency = time.monotonic() - started
            pump(.2)
            assert log.read_text().count('Graphical input admitted') == admitted, 'Hover still needed controller input'
            assert json.loads(ctl('view'))['cursor'] == view['cursor'], 'Hover changed row selection'
            print(json.dumps({'queue_hover_without_controller_input': True,
                              'hover_repaint_seconds': round(latency, 3)}))
        # Main header close target: x=960..989, y=7..30 at this viewport.
        close = b'\x1b[<0;976;20M\x1b[<0;976;20m'
        burst_started = time.monotonic()
        writer = None
        if args.motion_burst:
            packets = b''.join(f'\x1b[<35;{100 + n % 800};{60 + n % 100}M'.encode()
                               for n in range(1500)) + close
            def send_burst():
                # Pump output concurrently: a blocking PTY write while the
                # frontend writes frames would manufacture a deadlock.
                remaining = memoryview(packets)
                while remaining:
                    # Keep the kernel's small terminal input buffer from
                    # discarding the tail of an artificial single write.
                    remaining = remaining[os.write(master, remaining[:768]):]
                    time.sleep(.01)
            writer = threading.Thread(target=send_burst, daemon=True)
            writer.start()
        else:
            os.write(master, close)
        deadline = time.monotonic() + 8
        while child.poll() is None and time.monotonic() < deadline:
            pump(.1)
        assert child.poll() == 0, diagnostic()
        if writer:
            writer.join(timeout=.5)
            assert not writer.is_alive(), 'Motion sender did not finish'
            latency = time.monotonic() - burst_started
            assert latency < 2, f'Input accumulated behind hover positions: {latency:.3f}s'
            print(json.dumps({'queued_motion_reports': 1500, 'burst_then_click_seconds': round(latency, 3)}))
        pump(.1)
        assert b'\x1b[?1016l' in output, 'Pixel mouse mode was not restored'
        assert b'panicked at' not in output, bytes(output[-2000:])
        if args.imports:
            assert (root / 'data/imports.sqlite').is_file(), 'Import reviewer did not index staging'
            assert not (root / 'data/index.sqlite').exists(), 'Import preview created a permanent index'
            assert track.is_file(), 'Preview removed the staged album'
            assert not list(library.iterdir()), 'Preview moved music into the library without confirmation'
            print(json.dumps({'graphical_import_launch': True, 'separate_staging_index': True,
                              'preview_preserved_music': True}))
        print(json.dumps({'real_frontend_pty': True, 'zero_coordinates_survived': True,
                          'main_close_exits_frontend': True, 'pixel_mode_restored': True}))
    finally:
        # Every child belongs to this private process group, never a user session.
        # macOS can reject signalling an exited group leader. Enumerate only
        # this private group's remaining processes, owned by the test user.
        members = subprocess.check_output(['ps', '-axo', 'pid=,pgid=,uid='], text=True)
        for member in members.splitlines():
            pid, group, uid = map(int, member.split())
            if group == child.pid and uid == os.getuid():
                try:
                    os.kill(pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
        child.wait(timeout=5)
        os.close(master)
