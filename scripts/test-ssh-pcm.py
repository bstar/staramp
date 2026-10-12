#!/usr/bin/env python3
"""Exercise standalone SSH PCM with generated files and an isolated player.

The simulated frontend never opens a sound device. --ssh-host/--ssh-config
optionally run the same proof over a real SSH connection to this machine.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import queue
import shlex
import signal
import struct
import subprocess
import tempfile
import threading
import time

parser = argparse.ArgumentParser()
parser.add_argument('--staramp', required=True)
parser.add_argument('--ssh-host')
parser.add_argument('--ssh-config')
args = parser.parse_args()
amp = str(Path(args.staramp).resolve())

with tempfile.TemporaryDirectory(prefix='amp-pcm-', dir='/tmp') as temporary:
    root = Path(temporary)
    data = root / 'data'
    data.mkdir()
    (data / 'config.toml').write_text('volume = 1.0\n[rg]\nmode = "off"\n[eq]\nenabled = false\n')
    # Deliberately include bits float32 cannot retain.
    values = [-(2**31), -(2**31)+1, -16777217, -1, 0, 1, 16777217, 2**31-1]
    samples = (values * 48000)[:48000 * 2 * 4]
    payload = struct.pack('<' + 'i' * len(samples), *samples)
    song = root / 'precision.wav'
    song.write_bytes(b'RIFF' + struct.pack('<I', 36 + len(payload)) + b'WAVEfmt ' +
                    struct.pack('<IHHIIHH', 16, 1, 2, 48000, 48000*8, 8, 32) +
                    b'data' + struct.pack('<I', len(payload)) + payload)
    isolated = dict(STARAMP_DIR=str(data), STARAMP_CONFIG_DIR=str(data),
                    XDG_RUNTIME_DIR=str(root / 'runtime'),
                    DBUS_SESSION_BUS_ADDRESS='unix:path=/nonexistent/amp-pcm-proof',
                    ALSA_CONFIG_PATH=str(root / 'no-host-audio'))
    (root / 'no-host-audio').write_text('')
    env = dict(os.environ, **isolated)
    env.pop('SSH_CONNECTION', None)
    env.pop('SSH_TTY', None)

    def command(arguments):
        cmd = [amp] + arguments
        if args.ssh_host:
            remote = 'env ' + ' '.join(shlex.quote(k + '=' + v) for k, v in isolated.items()) + ' ' + shlex.join(cmd)
            cmd = ['ssh'] + (['-F', args.ssh_config] if args.ssh_config else []) + ['-T', '--', args.ssh_host, remote]
        return cmd

    def ctl(*arguments):
        result = subprocess.run(command(['ctl', *arguments]), env=env, capture_output=True, text=True, timeout=10)
        assert result.returncode == 0, result.stderr
        return result.stdout.strip()

    process = None
    server_pid = None
    server_pids = set()
    messages = None
    def attach(only=False, cells=True, name='proof', precise=True):
        global process, messages, server_pid
        cmd = ['--graphical-relay', name] + (['--attach-only'] if only else ['--directory', str(song)])
        process = subprocess.Popen(command(cmd), env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True)
        messages = queue.Queue()
        current = process
        target = messages
        def read():
            try:
                for line in current.stdout:
                    target.put(json.loads(line))
            finally:
                target.put({'type':'error', 'message':current.stderr.read() or 'Relay closed'})
        threading.Thread(target=read, daemon=True).start()
        send(type='hello', version=1, client='pcm-proof',
             viewport=dict(columns=100, rows=40, width=1000, height=800, generation=1),
             capabilities=dict(image_transport='none', pixel_geometry='estimated', pointer_precision='cells',
                 keyboard=True, paste=True, presentation_ack=True, native_surfaces=True, native_paths=True,
                 audio_pcm_v2=precise, local_media=False, cell_presentation=cells, presentation_switch=True))
        hello = wait(lambda m: m['type'] == 'hello')
        server_pid = int(hello['epoch'].split('-')[0])
        server_pids.add(server_pid)
        wait(lambda m: m['type'] == 'scene')

    def send(**message):
        process.stdin.write(json.dumps(message) + '\n')
        process.stdin.flush()

    def wait(predicate, timeout=8):
        deadline = time.monotonic() + timeout
        while True:
            message = messages.get(timeout=max(.01, deadline-time.monotonic()))
            if message['type'] == 'error':
                for log in root.rglob('staramp.log'):
                    print(log.read_text()[-3000:])
                raise AssertionError(message)
            if message['type'] == 'scene':
                scene = message['scene']
                send(type='presented', revision=scene['revision'], generation=scene['viewport']['generation'])
            if predicate(message):
                return message

    def media(kind):
        return wait(lambda m: m['type'] == 'media' and m['message']['kind'] == kind)['message']

    def key(code, number):
        send(type='input', id=number, revision=0, generation=1,
             input=dict(kind='key', code=code, modifiers=0))
        assert wait(lambda m: m['type'] == 'ack' and m['id'] == number)['accepted']

    def credit(opened, count):
        send(type='media', message=dict(kind='audio_credit', session=opened['session'], epoch=opened['epoch'], blocks=count))

    try:
        attach()
        key('char: ', 1)
        opened = media('pcm_open')
        assert opened['spec']['sample_rate'] == 48000
        assert opened['spec']['channels'] == 2
        assert opened['spec']['format'] == 'i32', opened
        assert not opened['compatibility']
        credit(opened, 8)
        blocks = [media('pcm_chunk') for _ in range(8)]
        raw = b''.join(base64.b64decode(b['data']) for b in blocks)
        assert raw == payload[:len(raw)], '32-bit PCM changed in transport'
        assert all(b['epoch'] == opened['epoch'] for b in blocks)
        # Exhausted credits must not prevent controls or disconnect cleanup.
        assert ctl('pause') == 'ok'
        closed = media('pcm_close')
        assert closed['epoch'] == opened['epoch']
        assert json.loads(ctl('status'))['state'] == 'paused'
        assert ctl('seek', '1') == 'ok'
        assert ctl('play') == 'ok'
        newer = media('pcm_open')
        assert newer['epoch'] > opened['epoch']
        credit(newer, 1)
        assert media('pcm_chunk')['epoch'] == newer['epoch']
        assert ctl('volume', '0.5') == 'ok'
        for _ in range(32):
            credit(newer, 1)
            block = media('pcm_chunk')
            assert block['epoch'] == newer['epoch'], 'DSP reopened the output device'
            if block['processed']:
                assert block['format'] == 'f64', block
                raw = base64.b64decode(block['data'])
                scaled = struct.unpack('<' + 'd' * (len(raw)//8), raw)
                assert all(v in {n / 4294967296 for n in values} for v in scaled)
                break
        else:
            raise AssertionError('Volume did not reach the float64 processing path')
        # Dropping the relay pauses its persistent owner.
        process.stdin.close()
        process.wait(timeout=5)
        deadline = time.monotonic()+5
        while json.loads(ctl('status'))['state'] != 'paused':
            assert time.monotonic() < deadline, 'Disconnect did not pause owner'
            time.sleep(.02)
        attach(only=True, cells=False)
        assert json.loads(ctl('status'))['state'] == 'paused'
        assert ctl('audio-compatibility', 'on') == 'ok'
        assert ctl('play') == 'ok'
        resumed = media('pcm_open')
        assert resumed['compatibility']
        credit(resumed, 1)
        assert media('pcm_chunk')['epoch'] == resumed['epoch']
        assert ctl('stop') == 'ok'
        media('pcm_close')
        assert ctl('volume', '1') == 'ok'
        key('char: ', 3)
        final = media('pcm_open')
        credit(final, 8)
        received = bytearray()
        while True:
            event = wait(lambda m: m['type'] == 'media' and m['message']['kind'] in ('pcm_chunk', 'pcm_end'))['message']
            assert event['epoch'] == final['epoch']
            if event['kind'] == 'pcm_end':
                break
            received.extend(base64.b64decode(event['data']))
            send(type='media', message=dict(kind='pcm_progress', session=final['session'], epoch=final['epoch'], frames=len(received)//8))
            credit(final, 1)
        assert received == payload, 'End of track lost or changed PCM samples'
        assert json.loads(ctl('status'))['state'] == 'playing', 'Stopped before local drain completed'
        send(type='media', message=dict(kind='pcm_finished', session=final['session'], epoch=final['epoch']))
        media('pcm_close')
        assert json.loads(ctl('status'))['state'] == 'stopped'
        process.stdin.close()
        process.wait(timeout=5)
        # A second controller uses the original owner via IPC.
        attach(name='mirror', cells=True)
        assert json.loads(ctl('status'))['state'] == 'stopped'
        key('char: ', 1)
        mirrored = media('pcm_open')
        credit(mirrored, 1)
        block = media('pcm_chunk')
        assert base64.b64decode(block['data']) == payload[:len(base64.b64decode(block['data']))]
        send(type='media', message=dict(kind='audio_error', session=mirrored['session'], epoch=mirrored['epoch'], message='Local audio: isolated device failure'))
        media('pcm_close')
        assert json.loads(ctl('status'))['state'] == 'paused', 'Local device failure did not pause owner'
        key('char:q', 4)
        assert process.wait(timeout=5) == 0
        attach(only=True, precise=False)
        assert json.loads(ctl('status'))['state'] == 'paused', 'Legacy frontend did not pause owner'
        key('char:q', 5)
        assert process.wait(timeout=5) == 0
        server_pid = None
        print('Standalone PCM: exact 32-bit bytes, float64 DSP, credits, pause, seek, disconnect, reattach, both presentations, owner mirroring, device failure, legacy frontend, explicit conversion, complete EOF drain and shutdown passed')
    finally:
        if process and process.poll() is None:
            process.kill()
            process.wait()
        for server_pid in server_pids:
            try:
                os.kill(server_pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
