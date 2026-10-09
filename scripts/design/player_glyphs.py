#!/usr/bin/env python3
"""Rasterize option 01's original clock and transport artwork at 1x/2x.

Clock atlases retain ten subpixel phases because the reference advances digits
by 34.1 design pixels. Nothing rescales these assets at runtime.
"""
from pathlib import Path
import subprocess
import xml.etree.ElementTree as ET
from winamp import Classic

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "assets/skins/classic"


def export(name, width, height, nodes):
    svg = f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}">{"".join(nodes)}</svg>'
    (OUT / "source" / f"{name}.svg").write_text(svg)
    for density in (1, 2):
        subprocess.run([
            "rsvg-convert", "-w", str(width * density), "-h", str(height * density),
            "-o", str(OUT / f"{density}x" / f"{name}.png"),
        ], input=svg.encode(), check=True)


for active, name in ((True, "clock-on"), (False, "clock-off")):
    nodes = []
    for phase in range(10):
        for index, character in enumerate("0123456789:"):
            drawing = Classic()
            drawing.nodes = []
            drawing.digit(index * 35 + phase / 10, phase * 54, character, 1.1)
            for source in drawing.nodes:
                node = ET.fromstring(source)
                if (node.get("fill") == drawing.p["accent"]) == active:
                    node.set("fill", "#ffffff")
                    nodes.append(ET.tostring(node, encoding="unicode"))
    export(name, 365, 540, nodes)

nodes = []
for index, label in enumerate(("|◀", "▶", "Ⅱ", "■", "▶|")):
    drawing = Classic()
    drawing.nodes = []
    drawing.button(index * 29, 0, 29, label, square=True)
    for source in drawing.nodes:
        node = ET.fromstring(source)
        if node.get("fill") == "#e0e2d5":
            node.set("fill", "#ffffff")
            nodes.append(ET.tostring(node, encoding="unicode"))
export("transport-glyphs", 145, 29, nodes)

# Static lettering is rasterized by the same font renderer as the study.
# Variable music metadata remains shaped Unicode text.
from html import escape
import math
labels = [
    (text, 12, True, 17, 24) for text in
    ['S T A R / A M P', 'PARAMETRIC EQUALIZER', 'ALBUM', 'ACTIVITY', 'PLAYLIST']
] + [
    (text, 14, True, 20, 29) for text in
    ['SHUFFLE', 'REP ALL', 'REP ONE', 'REP OFF', 'ON', 'OFF', 'IMPORT',
     'EXPORT', 'SAVE AS', 'BYPASS', 'DELETE', 'CHOOSE', 'RETRY', 'RETRY FAILED',
     'ADD', 'REM', 'SEL', 'MISC', 'LIST', 'TYPE', 'CHANNELS', 'MOVE UP', 'MOVE DOWN',
     'PREV', 'NEXT', '+ FILTER', 'PLAYLISTS', 'LIBRARY']
]
nodes=[]
metadata=[]
for index,(text,size,bold,baseline,height) in enumerate(labels):
    width=math.ceil(len(text)*size*.6)+2
    y=index*32
    nodes.append(f'<text x="0" y="{y+baseline}" fill="#ffffff" font-family="JetBrains Mono,monospace" font-size="{size}" font-weight="{700 if bold else 400}">{escape(text)}</text>')
    metadata.append((text,y,width,height))
export('fixed-labels',320,len(labels)*32,nodes)
import json
(OUT/'fixed-labels.json').write_text(json.dumps(metadata,indent=2)+'\n')
