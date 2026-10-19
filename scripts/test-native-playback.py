#!/usr/bin/env python3
"""Exercise native embed playback with silent audio on a real output device.

This optional local gate requires a working audio session. CI's device-free
embed tests cannot prove that dynamically loaded output plugins are installed.
All media/configuration are private temporary files; no audible signal is played.
"""
import argparse
import json
import os
from pathlib import Path
import queue
import re
import subprocess
import tempfile
import threading
import time
import wave


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='amp-playback-') as temporary:
        root = Path(temporary)
        media = root / 'silent.wav'
        with wave.open(str(media), 'wb') as audio:
            audio.setnchannels(2)
            audio.setsampwidth(2)
            audio.setframerate(44100)
            audio.writeframes(bytes(44100 * 4 * 10))
        with (root / 'stderr').open('w+') as errors:
            child = subprocess.Popen([str(Path(args.binary).resolve()), 'embed', '--stdio'],
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors,
                                     text=True, env=dict(os.environ, STARAMP_DIR=str(root/'data'),
                                                        STARAMP_CONFIG_DIR=str(root/'config')))
            messages = queue.Queue()

            def read():
                for line in child.stdout:
                    messages.put(json.loads(line))
            threading.Thread(target=read, daemon=True).start()

            def send(message):
                child.stdin.write(json.dumps(message) + '\n')
                child.stdin.flush()

            def until(predicate):
                deadline = time.monotonic() + 8
                while time.monotonic() < deadline:
                    message = messages.get(timeout=max(.01, deadline-time.monotonic()))
                    assert message['type'] != 'error', message.get('message')
                    if predicate(message):
                        return message
                raise AssertionError('Playback response timed out')

            def clock(frame):
                # The main seven-segment clock is geometry, while the seek
                # row always supplies an elapsed timestamp as shaped text.
                return next((node['text'] for node in frame.get('surface', {}).get('nodes', [])
                             if node.get('kind') == 'text'
                             and re.fullmatch(r'\d+:\d{2}', node.get('text', ''))), None)

            def click(frame, action):
                rect = next(hit['rect'] for hit in frame['surface']['hits'] if hit['action'] == action)
                cells = [(x,y) for x in range(frame['width']) for y in range(frame['height'])
                         if rect['x'] <= x*10+5 < rect['x']+rect['width']
                         and rect['y'] <= y*20+10 < rect['y']+rect['height']]
                assert cells, f'Unreachable native {action} button'
                x,y = cells[len(cells)//2]
                send({'type':'pointer','x':x,'y':y,'button':'left'})

            def toggle(frame, expected_icon):
                hits = [hit for hit in frame['surface']['hits']
                        if hit['action'] in ('play', 'pause')]
                assert len(hits) == 1, 'Play/Pause must share one button'
                assert has_icon(frame, expected_icon), f'Missing {expected_icon} icon'
                click(frame, hits[0]['action'])
                return hits[0]['rect']

            def has_icon(frame, name):
                return any(node.get('kind') == 'icon' and node.get('name') == name
                           for node in frame.get('surface', {}).get('nodes', []))

            try:
                until(lambda m: m['type'] == 'hello')
                theme = dict(bg=[10,12,14],fg=[220,220,220],muted=[120,120,120],accent=[80,170,230],
                             selected=[40,60,80],border=[100,100,100],error=[240,80,80])
                for generation, rows in enumerate((10, 8), start=1):
                    send(dict(type='configure',generation=generation,width=100,height=rows,
                              focused=True,theme=theme,native_surface=True,
                              graphics=dict(cell_width=10,cell_height=20)))
                    send(dict(type='play',generation=generation,paths=[str(media)],index=0))
                    until(lambda m: m['type'] == 'status' and m['playing'] and not m['paused'])
                    frame = until(lambda m: m['type'] == 'frame' and m['generation'] == generation
                                  and clock(m) == '0:01' and has_icon(m, 'media-pause'))
                    slot = toggle(frame, 'media-pause')
                    until(lambda m: m['type'] == 'status' and m['paused'])
                    frame = until(lambda m: m['type'] == 'frame' and has_icon(m, 'media-play'))
                    assert toggle(frame, 'media-play') == slot, 'Toggle moved when paused'
                    until(lambda m: m['type'] == 'status' and m['playing'] and not m['paused'])
                    send(dict(type='control',action='seek',value=5))
                    frame = until(lambda m: m['type'] == 'frame' and clock(m) == '0:06')
                    click(frame, 'stop')
                    until(lambda m: m['type'] == 'status' and not m['playing'])
                print('Full and compact playback: one Play/Pause toggle, clock, stop and seeking passed')
            finally:
                if child.poll() is None:
                    send(dict(type='shutdown'))
                    child.stdin.close()
                    try:
                        child.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait()
                errors.seek(0)
                diagnostics = errors.read()
                if diagnostics:
                    print(diagnostics)
            assert child.returncode == 0


if __name__ == '__main__':
    main()
