#!/usr/bin/env python3
"""Exercise greedy-cache visualizer timing through an isolated playback owner.

No terminal, audio hardware, network, user library or user configuration is used.
Run: python3 scripts/test-ssh-visualizer.py target/release/staramp
"""
import argparse
import base64
import json
import math
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import time
import wave

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary')
args = parser.parse_args()
binary = str(Path(args.binary).resolve())
with tempfile.TemporaryDirectory(prefix='amp-vis-', dir='/tmp') as temporary:
    root = Path(temporary)
    library = root / 'music'
    library.mkdir()
    config = root / 'config'
    config.mkdir()
    (config / 'config.toml').write_text('library_root = ' + json.dumps(str(library)) + '\n[vis]\nmode = "leds"\n')
    rate = 44100
    with wave.open(str(library / 'tones.wav'), 'wb') as output:
        output.setparams((1, 2, rate, 0, 'NONE', 'not compressed'))
        for hz in (220, 4000, 0):
            output.writeframes(b''.join(struct.pack('<h', int(12000 * math.sin(math.tau * hz * n / rate)))
                                        for n in range(rate * 2)))
    env = dict(os.environ, STARAMP_DIR=str(root / 'data'), STARAMP_CONFIG_DIR=str(config),
               XDG_RUNTIME_DIR=str(root / 'runtime'), STAR_GRAPHICS_SYSTEM_FONTS='0',
               DBUS_SESSION_BUS_ADDRESS='unix:path=/nonexistent/staramp-test-bus')
    (root / 'runtime').mkdir()
    log = (root / 'server.log').open('w+')
    child = subprocess.Popen([binary, '--graphical-server', 'regression'], env=env,
                             stdin=subprocess.DEVNULL, stdout=log, stderr=log)
    def ctl(request):
        reply = subprocess.run([binary, 'ctl', request], env=env, capture_output=True,
                               text=True, timeout=5)
        if reply.returncode or reply.stdout.startswith('error:'):
            raise RuntimeError(reply.stdout + reply.stderr)
        return reply.stdout.strip()
    def snapshot():
        return json.loads(ctl('mirror'))
    def quiet(state):
        return not state['bands'].strip('0')
    try:
        deadline = time.monotonic() + 10
        while True:
            try:
                snapshot()
                break
            except RuntimeError:
                assert child.poll() is None, 'isolated playback owner exited'
                assert time.monotonic() < deadline, 'isolated playback owner did not become ready'
                time.sleep(.05)
        token = 314159
        assert ctl(f'audio-route {token} cache') == 'ok'
        assert ctl('set-queue ["tones.wav"]') == '1'
        assert ctl('play-uri tones.wav') == 'ok'
        frames = 0
        epoch = None
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            poll = json.loads(ctl(f'audio-pcm {token}'))
            packet = poll.get('packet')
            if packet:
                epoch = packet['epoch']
                spec = packet['spec']
                sizes = {'i16': 2, 'i24': 3, 'i32': 4, 'f32': 4, 'f64': 8}
                size = sizes[packet['format'].lower()]
                frames += len(base64.b64decode(packet['data'])) // (size * spec['channels'])
                ctl(f'audio-buffered {token} {epoch} {frames}')
            elif poll['finished'] and epoch is not None:
                break
            else:
                time.sleep(.01)
        assert frames == rate * 6, f'whole song was not greedily downloaded: {frames}'
        time.sleep(.15)
        assert quiet(snapshot()), 'visualizer animated before the device played any audio'
        print('PASS: entire song cached while playback clock is zero; visualizer stays clear')
        peaks = []
        for start in (rate, rate * 3):
            for offset in range(0, rate // 2, rate // 10):
                ctl(f'audio-progress {token} {epoch} {start + offset}')
                time.sleep(.1)
            bands = snapshot()['bands']
            values = [int(bands[i:i + 2]) for i in range(0, len(bands), 2)]
            assert values and max(values) > 10, 'fully cached playback produced a flat spectrum'
            peaks.append(values.index(max(values)))
        assert peaks[1] > peaks[0], 'spectrum followed the download tail instead of audible tones'
        print('PASS: spectrum follows audible frequency changes after complete download')
        ctl('pause')
        time.sleep(.2)
        assert quiet(snapshot()), 'paused playback kept animating'
        print('PASS: paused playback clears the spectrum')
        ctl('play')
        ctl(f'audio-progress {token} {epoch} {rate * 4 - 1}')
        time.sleep(.15)
        assert not quiet(snapshot()), 'resume did not restore the spectrum'
        time.sleep(.5)
        assert quiet(snapshot()), 'stalled device clock kept animating'
        print('PASS: resume restores the spectrum; stalled playback clears it')
    finally:
        child.terminate()
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait()
        if child.returncode not in (0, -15):
            log.seek(0)
            print(log.read()[-4000:])
        log.close()
