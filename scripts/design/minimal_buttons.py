#!/usr/bin/env python3
"""Original 1x/2x lettering for Minimal action buttons; transport is unchanged."""
import html
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'assets/skins/classic'


def export(name, width, height, body):
    source = OUT / 'source' / f'{name}.svg'
    source.write_text(f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">{body}</svg>\n')
    for density in (1, 2):
        subprocess.run(['rsvg-convert', '-w', str(width * density), '-h', str(height * density), str(source), '-o', str(OUT / f'{density}x' / f'{name}.png')], check=True)


labels = json.loads((OUT / 'fixed-labels.json').read_text())[5:]
# EQ profile controls use the same lettering as the rest of the Minimal buttons.
labels += [(label, 0, 0, 0) for label in
           ['SAVE', 'LOAD', 'NEW', 'RENAME', 'REVERT', 'CANCEL', 'APPLY']]
rows, body = [], []
for index, (label, _, _, _) in enumerate(labels):
    width = len(label) * 7 + 2
    rows.append([label, index * 24, width, 22])
    body.append(f'<text x="0" y="{index * 24 + 15}" fill="white" font-family="JetBrains Mono,monospace" font-size="11" font-weight="700">{html.escape(label)}</text>')
export('minimal-labels', 128, len(labels) * 24, ''.join(body))
(OUT / 'minimal-labels.json').write_text(json.dumps(rows, indent=2) + '\n')
