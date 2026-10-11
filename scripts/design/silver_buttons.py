#!/usr/bin/env python3
"""Original silver skin artwork, independently rasterized at 1x and 2x."""
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'assets/skins/winamp-silver'
for directory in ('source', '1x', '2x'):
    (OUT / directory).mkdir(parents=True, exist_ok=True)

# Thin Outline (study D): a single muted silver rim, without bevel strokes.
for rigid in (False, True):
    name = 'panel-rigid' if rigid else 'panel-rounded'
    body = f'''<rect width="24" height="24" rx="{0 if rigid else 4}" fill="#60657c"/>
      <rect x="1" y="1" width="22" height="22" rx="{0 if rigid else 3}" fill="#28283b"/>'''
    source = OUT / 'source' / f'{name}.svg'
    source.write_text(f'<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24">{body}</svg>\n')
    for density in (1, 2):
        subprocess.run(['rsvg-convert', '-w', str(24 * density), '-h', str(24 * density),
                        str(source), '-o', str(OUT / f'{density}x' / f'{name}.png')], check=True)

for state in ('normal', 'hover', 'pressed', 'focus', 'active', 'disabled'):
    pressed = state == 'pressed'
    disabled = state == 'disabled'
    # Square, machined faces: a narrow tonal range and one hard pixel bevel.
    stops = ('#94a1ad', '#a1aeb9', '#acb8c2') if pressed else (
        ('#d5dde4', '#c8d3dc', '#b8c4ce') if state == 'hover' else
        ('#818b97', '#77838f', '#6b7785') if disabled else
        ('#c8d2da', '#b9c4cd', '#a5b1bd'))
    hi, lo = ('#45505f', '#dce4eb') if pressed else ('#e0e7ec', '#45505f')
    body = f'''<defs><linearGradient id="face" x1="0" y1="0" x2="0" y2="1">
      <stop stop-color="{stops[0]}"/><stop offset=".48" stop-color="{stops[1]}"/>
      <stop offset="1" stop-color="{stops[2]}"/></linearGradient></defs>
      <rect x="0" y="0" width="29" height="29" fill="#10141e"/>
      <rect x="1" y="1" width="27" height="27" fill="{lo}"/>
      <rect x="1" y="1" width="27" height="1" fill="{hi}"/>
      <rect x="1" y="1" width="1" height="27" fill="{hi}"/>
      <rect x="2" y="2" width="25" height="25" fill="url(#face)"/>'''
    if state in ('focus', 'active'):
        body += '<rect x="5" y="26" width="19" height="1" fill="#6b985b"/>'
    if state == 'focus':
        body += '<path d="M0.5 28.5V0.5H28.5V28.5Z" fill="none" stroke="#cfd8e1"/>'
    source = OUT / 'source' / f'button-{state}.svg'
    source.write_text(f'<svg xmlns="http://www.w3.org/2000/svg" width="29" height="29" viewBox="0 0 29 29">{body}</svg>\n')
    for density in (1, 2):
        subprocess.run(['rsvg-convert', '-w', str(29 * density), '-h', str(29 * density),
                        str(source), '-o', str(OUT / f'{density}x' / f'button-{state}.png')], check=True)
