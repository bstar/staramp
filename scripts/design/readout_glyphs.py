#!/usr/bin/env python3
"""Original fixed-metric player readouts at 1x/2x, independent of terminal fonts."""
import json
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parents[2]
out = root / 'assets/skins/classic'
characters = '0123456789:VOL'
cells, nodes = [], []
for row, (size, advance) in enumerate([(11, 7), (13, 8)]):
    for column, character in enumerate(characters):
        x, y = column * 12, row * 24
        cells.append(dict(size=size, character=character, x=x, y=y,
                          width=12, height=24, advance=advance))
        nodes.append(f'<text x="{x}" y="{y + 17}" fill="white" '
                     f'font-family="JetBrains Mono,monospace" font-size="{size}">{character}</text>')
source = out / 'source/readout-glyphs.svg'
source.write_text(f'<svg xmlns="http://www.w3.org/2000/svg" width="{len(characters) * 12}" '
                  f'height="48">{"".join(nodes)}</svg>\n')
for density in (1, 2):
    subprocess.run(['rsvg-convert', '-w', str(len(characters) * 12 * density),
                    '-h', str(48 * density), str(source), '-o',
                    str(out / f'{density}x/readout-glyphs.png')], check=True)
(out / 'readout-glyphs.json').write_text(json.dumps(cells, indent=2) + '\n')
