#!/usr/bin/env python3
"""Editable native-pane layout studies; no changes to the running application."""
import argparse
import base64
import html
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'docs/graphical/treatments'
FONT = 'JetBrains Mono, JetBrainsMono Nerd Font, monospace'
P = dict(bg='#171820', panel='#303340', title='#242733', ink='#d7d9cf', dim='#989eac', accent='#a4d791', border='#747986', rule='#474d5d', hover='#383d49', error='#e09885', queued='#d7b579')


def uri(path):
    mime = 'image/png' if path.suffix == '.png' else 'image/jpeg'
    return f'data:{mime};base64,' + base64.b64encode(path.read_bytes()).decode()


class Drawing:
    def __init__(self, cover):
        self.nodes = []
        self.cover = cover

    def rect(self, x, y, w, h, color, stroke='none', cls=''):
        self.nodes.append(f'<rect class="{cls}" x="{x}" y="{y}" width="{w}" height="{h}" fill="{P.get(color, color)}" stroke="{P.get(stroke, stroke)}"/>')

    def text(self, x, y, value, color='ink', size=12, bold=False, anchor='start', cls=''):
        self.nodes.append(f'<text class="{cls}" x="{x}" y="{y}" fill="{P.get(color, color)}" font-size="{size}" font-weight="{700 if bold else 400}" text-anchor="{anchor}" dominant-baseline="middle">{html.escape(value)}</text>')

    def line(self, x, y, width, color='rule'):
        self.rect(x, y, width, 1, color)

    def button(self, x, y, width, label, action='', size=11):
        action = action or label.lower().replace(' ', '-')
        self.nodes.append(f'<g class="button" tabindex="0" role="button" aria-label="{html.escape(label)}" data-action="{action}">')
        self.rect(x, y - 3, width, 29, 'transparent', cls='target')
        self.rect(x + .5, y + .5, width - 1, 21, 'transparent', 'transparent', cls='face')
        self.text(x + width / 2, y + 11.5, label, size=size, bold=True, anchor='middle')
        self.nodes.append('</g>')

    def chrome(self, title):
        self.rect(.5, .5, 591, 319, 'panel', 'border')
        self.line(2, 2, 588, '#555b69')
        self.rect(6, 6, 580, 24, 'title')
        self.text(17, 18, title, '#bbc6d5', 11, True)
        self.button(490, 7, 62, 'settings', size=10)
        self.button(559, 7, 22, '×', size=19)

    def artwork(self, x, y, width):
        height = round((width - 4) * 264 / 300) + 4
        self.rect(x, y, width, height, 'border')
        if self.cover:
            self.nodes.append(f'<image x="{x+2}" y="{y+2}" width="{width-4}" height="{height-4}" href="{self.cover}" preserveAspectRatio="xMidYMid meet"/>')
        else:
            self.rect(x + 2, y + 2, width - 4, height - 4, 'title')
            self.text(x + width / 2, y + height / 2, 'ALBUM ART', 'dim', 11, anchor='middle')
        return height

    def artwork_controls(self, x, y, width):
        self.button(x, y, 22, '‹', 'art-prev', 14)
        self.text(x + width / 2, y + 11, '1 / 6', 'dim', 11, anchor='middle', cls='art-index')
        self.button(x + width - 22, y, 22, '›', 'art-next', 14)

    def footer(self, left, action, label, width=118):
        self.line(16, 277, 560)
        self.text(16, 298, left, 'dim', 10)
        self.button(576 - width, 287, width, label, action)

    def svg(self, title):
        return f'<svg xmlns="http://www.w3.org/2000/svg" width="592" height="320" viewBox="0 0 592 320" aria-label="{html.escape(title)}"><style>svg{{font-family:{FONT};shape-rendering:geometricPrecision}}.button{{cursor:pointer;outline:none}}.button:hover .face{{fill:#383d49;stroke:#555c6b}}.button:active .face{{fill:#242733;stroke:#11131a}}.button:focus-visible .face{{stroke:#a4d791}}</style>'+''.join(self.nodes)+'</svg>'


OPTIONS = [
    ('a', 'Refined classic', 'Familiar proportions, clearer hierarchy, and a useful service strip above the listening history.'),
    ('b', 'Compact ledger', 'Aligned facts, a 160px cover with its selector beside it, and a denser Activity list.'),
    ('c', 'Artwork first', 'A larger cover anchors Album; Activity gives the current listen its own space.'),
    ('d', 'Split rail', 'Dedicated artwork and service columns keep secondary controls together and details easy to scan.'),
]

LISTENS = [
    ('Wasp / Behind the Wall of Sleep', 'Black Sabbath · 2:12 listened', 'PAUSED', 'accent'),
    ('Behind the Wall of Sleep', 'Black Sabbath · 3:11 · completed', 'SENT', 'dim'),
    ('Black Sabbath', 'Black Sabbath · 6:18 · completed', 'QUEUED', 'queued'),
    ('The Wizard', 'Black Sabbath · 4:24 · completed', 'ERROR', 'error'),
]


def catalog(d):
    d.button(332, 7, 72, 'artist ↗', size=10)
    d.button(406, 7, 72, 'album ↗', size=10)


def album_footer(d):
    d.line(16, 277, 560)
    d.text(16, 298, 'Embedded front cover', 'dim', 10)
    d.button(384, 287, 80, 'CHOOSE')
    d.button(480, 287, 80, 'RETRY')


def album(key, cover):
    d = Drawing(cover)
    d.chrome('ALBUM')
    catalog(d)
    if key == 'a':
        d.artwork(16, 54, 180)
        d.artwork_controls(16, 232, 180)
        d.text(220, 65, 'Black Sabbath', 'accent', 17, True)
        d.text(220, 95, 'Black Sabbath', size=14)
        d.text(220, 130, '1970 · 6 tracks · 42:53', 'dim', 12)
        d.text(220, 157, 'FLAC · 44.1 kHz · 16-bit', size=12)
        d.text(220, 192, 'GENRE', 'dim', 10)
        d.text(220, 212, 'Rock', size=12)
        d.text(220, 246, 'Open original ↗', 'dim', 10)
    elif key == 'b':
        d.artwork(16, 59, 160)
        d.artwork_controls(200, 182, 152)
        d.text(200, 64, 'Black Sabbath', 'accent', 16, True)
        d.text(200, 91, 'Black Sabbath', size=13)
        d.text(200, 127, 'Rock', 'dim', 12)
        d.text(200, 155, 'FLAC · 44.1 kHz · 16-bit', size=11)
        d.line(16, 220, 560)
        for x, name, value in [(16, 'RELEASED', '1970'), (210, 'TRACKS', '6'), (404, 'DURATION', '42:53')]:
            d.text(x, 235, name, 'dim', 10)
            d.text(x, 258, value, size=14)
    elif key == 'c':
        d.artwork(16, 50, 220)
        d.artwork_controls(16, 247, 220)
        d.text(260, 67, 'Black Sabbath', 'accent', 20, True)
        d.text(260, 101, 'Black Sabbath', size=14)
        d.text(260, 141, '1970', size=16)
        d.text(260, 170, '6 tracks · 42:53', 'dim', 12)
        d.text(260, 210, 'Rock', 'dim', 12)
        d.text(260, 241, 'FLAC · 44.1 kHz · 16-bit', size=11)
    else:
        d.rect(215, 45, 1, 219, 'rule')
        d.artwork(16, 54, 180)
        d.artwork_controls(16, 232, 180)
        d.text(238, 62, 'ALBUM', 'dim', 9)
        d.text(238, 85, 'Black Sabbath', 'accent', 16, True)
        d.text(238, 119, 'ARTIST', 'dim', 9)
        d.text(238, 141, 'Black Sabbath', size=13)
        for x, name, value in [(238, 'YEAR', '1970'), (354, 'TRACKS', '6'), (470, 'TIME', '42:53')]:
            d.text(x, 177, name, 'dim', 9)
            d.text(x, 198, value, size=12)
        d.text(238, 231, 'Rock', 'dim', 11)
        d.text(238, 253, 'FLAC · 44.1 kHz · 16-bit', size=11)
    album_footer(d)
    return d.svg(f'{key.upper()} — Album')


def services(d, y=51, compact=False):
    for x, title, status, color in [(16, 'LOCAL HISTORY', 'Recording', 'accent'), (210, 'LAST.FM', 'Enabled', 'accent'), (404, 'LISTENBRAINZ', 'Off', 'dim')]:
        d.text(x, y, title, 'dim', 9, True)
        d.text(x, y + 19, status, color, 11)
    if not compact:
        d.line(16, y + 39, 560)


def listen(d, x, y, width, row, details=True):
    title, detail, status, color = row
    d.text(x, y, title, size=11 if width > 400 else 10)
    d.text(x + width, y, status, color, 9, True, anchor='end')
    if details:
        d.text(x, y + 16, detail, 'dim', 10 if width > 400 else 9)


def activity(key):
    d = Drawing(None)
    d.chrome('ACTIVITY')
    if key == 'a':
        services(d)
        d.text(16, 110, 'RECENT LISTENS', 'dim', 9, True)
        d.text(576, 110, 'DELIVERY', 'dim', 9, True, 'end')
        for i, row in enumerate(LISTENS):
            listen(d, 16, 133 + i * 35, 560, row)
    elif key == 'b':
        services(d, 50, True)
        d.line(16, 87, 560)
        for x, label in [(16, 'LISTENED'), (96, 'TRACK'), (490, 'DELIVERY')]:
            d.text(x, 106, label, 'dim', 9, True)
        for i, (time, row) in enumerate(zip(['2:12', '3:11', '6:18', '4:24', '5:35', '2:29'], LISTENS + [('N.I.B.', '', 'SENT', 'dim'), ('Sleeping Village', '', 'LOCAL', 'dim')])):
            y = 132 + i * 23
            d.text(16, y, time, 'dim', 11)
            d.text(96, y, row[0], size=11)
            d.text(576, y, row[2], row[3], 9, True, 'end')
            if i < 5:
                d.line(16, y + 11, 560, '#393d4a')
    elif key == 'c':
        d.text(16, 53, 'CURRENT LISTEN', 'dim', 9, True)
        d.text(576, 53, 'PAUSED', 'accent', 10, True, 'end')
        d.text(16, 81, 'Wasp / Behind the Wall of Sleep', size=16, bold=True)
        d.text(16, 105, 'Bassically / N.I.B.', size=16, bold=True)
        d.text(16, 131, 'Black Sabbath · 2:12 listened', 'dim', 11)
        d.line(16, 150, 560)
        d.text(16, 169, 'PREVIOUS LISTENS', 'dim', 9, True)
        for i, row in enumerate(LISTENS[1:3]):
            listen(d, 16, 191 + i * 36, 560, row)
        d.text(16, 263, 'Local: recording · Last.fm: on · ListenBrainz: off', 'dim', 10)
    else:
        d.rect(157, 45, 1, 219, 'rule')
        for y, label, state, color in [(55, 'LOCAL HISTORY', 'Recording', 'accent'), (125, 'LAST.FM', 'Enabled', 'accent'), (195, 'LISTENBRAINZ', 'Off', 'dim')]:
            d.text(16, y, label, 'dim', 9, True)
            d.text(16, y + 22, state, color, 12)
            if y > 55:
                d.button(10, y + 37, 72, 'SETTINGS', size=9)
        d.text(178, 55, 'RECENT LISTENS', 'dim', 9, True)
        for i, row in enumerate(LISTENS):
            # The narrow rail layout uses shorter titles without hiding the
            # full current medley in the detail line.
            if i == 0:
                row = ('Wasp / Behind the Wall of Sleep', 'Bassically / N.I.B. · 2:12 listened', row[2], row[3])
            listen(d, 178, 85 + i * 47, 398, row)
    d.footer('2 pending · 1 needs attention', 'retry-failed', 'RETRY FAILED')
    return d.svg(f'{key.upper()} — Activity')


def render(svg, filename, scale=1, width=1196, height=320):
    subprocess.run(['rsvg-convert', '-w', str(width * scale), '-h', str(height * scale), str(svg), '-o', str(filename)], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cover', type=Path)
    args = parser.parse_args()
    cover = uri(args.cover) if args.cover else None
    articles, overview = [], []
    for key, name, description in OPTIONS:
        left, right = album(key, cover), activity(key)
        for pane, svg in [('album', left), ('activity', right)]:
            (OUT / f'panes-{key}-{pane}.svg').write_text(svg)
        pair = '<svg xmlns="http://www.w3.org/2000/svg" width="1196" height="320" viewBox="0 0 1196 320">'+left+'<g transform="translate(604,0)">'+right+'</g></svg>'
        source = OUT / f'panes-{key}.svg'
        source.write_text(pair)
        for density in (1, 2):
            render(source, OUT / f'panes-{key}-{density}x.png', density)
        articles.append(f'<article id="option-{key}"><div class="caption"><div><h2>{key.upper()} — {name}</h2><p>{description}</p></div><a href="panes-{key}-1x.png">1× PNG ↗</a><a href="panes-{key}-2x.png">2× PNG ↗</a><a href="panes-{key}.svg">SVG ↗</a></div><div class="panes"><div class="pane album">{left}</div><div class="pane activity">{right}</div></div></article>')
        y = 80 + len(overview) * 385
        overview.append(f'<text x="24" y="{y-16}" fill="#d7d9cf" font-family="{FONT}" font-size="16">{key.upper()} — {name}</text><image x="24" y="{y}" width="1196" height="320" href="{uri(OUT / f"panes-{key}-1x.png")}"/>')
    source = OUT / 'panes-overview.svg'
    source.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="1244" height="1585"><rect width="1244" height="1585" fill="#171820"/><text x="24" y="30" fill="#d7d9cf" font-family="sans-serif" font-size="20">Album + Activity · layout treatments</text>'+''.join(overview)+'</svg>')
    render(source, OUT / 'panes-overview.png', width=1244, height=1585)
    page = '''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>STAR/AMP — Album and Activity</title><style>
    :root{color-scheme:dark;font-family:system-ui,sans-serif;background:#171820;color:#d7d9cf}*{box-sizing:border-box}body{margin:0}main{max-width:1260px;padding:28px 24px;margin:auto}h1{font-size:30px;letter-spacing:-.6px;margin:0 0 10px}h2{font-size:18px;margin:0 0 8px}p{color:#989eac;font-size:14px;line-height:1.55;margin:0 0 20px}a{color:#a4d791;text-underline-offset:3px}nav{display:flex;gap:18px;flex-wrap:wrap;margin:24px 0}.toolbar{display:flex;gap:20px;align-items:center;flex-wrap:wrap;padding:16px 0;border-top:1px solid #353946;border-bottom:1px solid #353946;margin-bottom:28px;font-size:13px}.toolbar label{display:flex;align-items:center;gap:10px}select{font:inherit;background:#242733;color:#d7d9cf;border:1px solid #747986;padding:6px 10px}article{margin:0 0 32px;padding:0 0 28px;border-bottom:1px solid #353946;scroll-margin:20px}.caption{display:flex;align-items:center;gap:18px;flex-wrap:wrap}.caption>div{flex:1;min-width:260px}.caption p{max-width:820px;margin-bottom:16px}.caption>a{font-size:12px}.panes{display:flex;gap:12px;overflow:auto}.pane{flex:0 0 592px;width:592px}.pane>svg{display:block;width:592px;height:320px}.only-album .activity,.only-activity .album{display:none}.note{font-size:12px}.demo-message{min-height:20px;color:#a4d791;font-size:12px;margin-top:12px}@media(max-width:1230px){.panes{flex-wrap:wrap;overflow:auto}}@media(max-width:650px){main{padding:20px 12px}h1{font-size:26px}.panes{flex-wrap:nowrap;display:block}.pane{margin-bottom:12px}.caption>div{min-width:0;flex-basis:100%}}
    </style><main><h1>Album + Activity</h1><p>Four layout directions with the current rack palette and Minimal action buttons. Every pane is 592 × 320px at 1:1 size. These are design studies with sample listening history and service states.</p><nav>'''+''.join(f'<a href="#option-{key}">{key.upper()} · {name}</a>' for key, name, _ in OPTIONS)+'''</nav><div class="toolbar"><label>Show <select id="pane-view"><option value="both">Both panes</option><option value="album">Album only</option><option value="activity">Activity only</option></select></label><a href="panes-overview.png">All four in one image ↗</a><a href="buttons.html">Button comparisons ↗</a><span>592 × 320 per pane · scroll on narrow screens</span></div>'''+''.join(articles)+'''<p class="note">Try the artwork arrows, or Tab through the controls to inspect focus. Selecting a layout here does not modify the native app.</p><div class="demo-message" role="status" aria-live="polite"></div></main><script>
    document.querySelector('#pane-view').addEventListener('change',e=>{document.querySelector('main').className=e.target.value==='both'?'':'only-'+e.target.value});
    document.querySelectorAll('.button').forEach(button=>{const activate=()=>{const action=button.dataset.action;const svg=button.closest('svg');if(action==='art-prev'||action==='art-next'){const label=svg.querySelector('.art-index');let n=parseInt(label.textContent);label.textContent=((n-1+(action==='art-next'?1:5))%6+1)+' / 6'}else{document.querySelector('.demo-message').textContent='Layout study · '+button.getAttribute('aria-label')}};button.addEventListener('click',activate);button.addEventListener('keydown',e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();activate()}})});
    </script></html>'''
    (OUT / 'panes.html').write_text(page)
    print(OUT / 'panes.html')


if __name__ == '__main__':
    main()
