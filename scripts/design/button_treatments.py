#!/usr/bin/env python3
"""Editable button studies; no changes to the running native application."""
import argparse
import base64
import html
from pathlib import Path
import subprocess

OUT = Path(__file__).resolve().parents[2] / 'docs/graphical/treatments'
FONT = 'JetBrains Mono, JetBrainsMono Nerd Font, monospace'
OPTIONS = [
    ('a', 'Compact Classic', 'Same beveled character, less bulk.', '#474d5d', '#12141b', '#959baa', '#171920'),
    ('b', 'Quiet bevel', 'Softer edges and a lower-contrast face.', '#3b404e', '#555c6b', '#686f7e', '#242733'),
    ('c', 'Thin outline', 'A single fine border, with an open face.', 'transparent', '#747986', 'none', 'none'),
    ('d', 'Minimal', 'Text and symbols; the frame appears on interaction.', 'transparent', 'transparent', 'none', 'none'),
]


def uri(path):
    mime = 'image/png' if path.suffix == '.png' else 'image/jpeg'
    return f'data:{mime};base64,' + base64.b64encode(path.read_bytes()).decode()


def rect(x, y, w, h, color, stroke='none', radius=0, cls=''):
    return f'<rect class="{cls}" x="{x}" y="{y}" width="{w}" height="{h}" rx="{radius}" fill="{color}" stroke="{stroke}"/>'


def text(x, y, value, color='#d7d9cf', size=11, bold=False, anchor='start', cls=''):
    return f'<text class="{cls}" x="{x}" y="{y}" fill="{color}" font-size="{size}" font-weight="{700 if bold else 400}" text-anchor="{anchor}" dominant-baseline="middle">{html.escape(value)}</text>'


def button(option, x, y, w, label, state='', icon=None, height=22, action=''):
    _, _, _, face, border, light, dark = option
    cx, cy = x + w / 2, y + height / 2
    action = action or label.lower().replace(' ', '-')
    parts = [f'<g class="button {state}" role="button" tabindex="{ -1 if state == "disabled" else 0}" aria-label="{html.escape(label)}" aria-disabled="{str(state == "disabled").lower()}" data-action="{action}">']
    # The visible control is smaller; a transparent target retains the old
    # 29-pixel vertical click area without altering the exported artwork.
    parts += [rect(x, y - 3, w, height + 7, 'transparent', cls='target')]
    parts += [rect(x + .5, y + .5, w - 1, height - 1, face, border, cls='face')]
    if light != 'none':
        parts += [f'<path class="light" d="M{x+1.5} {y+height-2} V{y+1.5} H{x+w-2}" fill="none" stroke="{light}"/>']
        parts += [f'<path class="dark" d="M{x+1.5} {y+height-1.5} H{x+w-1.5} V{y+1.5}" fill="none" stroke="{dark}"/>']
    parts += [rect(x + .5, y + .5, w - 1, height - 1, 'none', '#a4d791', cls='focus-ring')]
    if icon:
        paths = {
            'previous': f'M{cx+2} {cy-4} L{cx-3} {cy} L{cx+2} {cy+4} Z M{cx-5} {cy-4} V{cy+4}',
            'next': f'M{cx-2} {cy-4} L{cx+3} {cy} L{cx-2} {cy+4} Z M{cx+5} {cy-4} V{cy+4}',
            'play': f'M{cx-3} {cy-4} L{cx+4} {cy} L{cx-3} {cy+4} Z',
            'pause': f'M{cx-2.5} {cy-4} V{cy+4} M{cx+2.5} {cy-4} V{cy+4}',
            'stop': f'M{cx-3.5} {cy-3.5} H{cx+3.5} V{cy+3.5} H{cx-3.5} Z',
            'left': f'M{cx+1.5} {cy-3} L{cx-1.5} {cy} L{cx+1.5} {cy+3}',
            'right': f'M{cx-1.5} {cy-3} L{cx+1.5} {cy} L{cx-1.5} {cy+3}',
        }
        parts += [f'<path class="ink icon" d="{paths[icon]}" fill="{"#d7d9cf" if icon in ["play", "stop"] else "none"}" stroke="#d7d9cf" stroke-width="1.5" stroke-linejoin="round"/>']
    else:
        parts += [text(cx, cy + .5, label, bold=True, anchor='middle', cls='ink')]
    return ''.join(parts) + '</g>'


def study(option, cover):
    key, name, _, face, border, light, dark = option
    css = f'''
    svg{{font-family:{FONT};shape-rendering:geometricPrecision}}
    .button{{cursor:pointer;outline:none}}
    .focus-ring{{opacity:0;pointer-events:none}}
    .button.hover .face{{fill:#505665;stroke:#8b92a2}}
    .button:hover .face{{fill:#505665;stroke:#8b92a2}}
    .button.pressed .face{{fill:#242733;stroke:#11131a}}
    .button:active .face{{fill:#242733;stroke:#11131a}}
    .button.pressed .light{{stroke:#171920}}
    .button:active .light{{stroke:#171920}}
    .button.pressed .dark{{stroke:#747986}}
    .button:active .dark{{stroke:#747986}}
    .button.focused .focus-ring,.button.active .focus-ring{{opacity:1}}
    .button:focus-visible .focus-ring{{opacity:1}}
    .button.active .ink{{fill:#a4d791;stroke:#a4d791}}
    .button.active text.ink{{stroke:none}}
    .button.disabled{{opacity:.4;pointer-events:none}}
    .button.disabled .target{{pointer-events:none}}
    {'.button:hover .face{fill:#383d49;stroke:#555c6b}.button.hover .face{fill:#383d49;stroke:#555c6b}' if key == 'd' else ''}
    '''
    css = '\n'.join(line.replace('.button', f'.treatment-{key} .button').rstrip() for line in css.splitlines())
    n = [f'<svg xmlns="http://www.w3.org/2000/svg" class="treatment-{key}" width="592" height="464" viewBox="0 0 592 464" aria-label="{name} button study"><style>{css}</style>']
    n += [rect(0, 0, 592, 464, '#171820'), rect(.5, .5, 591, 286, '#303340', '#747986')]
    n += [rect(6, 6, 580, 24, '#242733'), text(17, 18, 'ALBUM', '#bbc6d5', bold=True)]
    n += [text(334, 18, 'artist ↗   album ↗   settings', '#989eac', 10), text(575, 18, '×', '#d7d9cf', 19, anchor='middle')]
    n += [rect(9, 53, 180, 159, '#747986')]
    if cover:
        n += [f'<image x="11" y="55" width="176" height="155" href="{cover}" preserveAspectRatio="xMidYMid meet"/>']
    else:
        n += [rect(11, 55, 176, 155, '#101813'), text(99, 132, 'ALBUM ART', '#989eac', anchor='middle')]
    n += [text(207, 61, 'Black Sabbath', '#a4d791', 14, True), text(207, 93, 'Black Sabbath', size=14)]
    n += [text(207, 119, '1970 · 6 tracks · 42:53', '#989eac', 12), text(207, 144, 'flac · 44.1 kHz · 16-bit', size=12)]
    n += [text(207, 168, 'GENRE  Rock', '#989eac'), rect(207, 182, 373, 1, '#474d5d'), text(207, 201, 'ARTWORK  embedded', '#a4d791')]
    n += [button(option, 10, 244, 22, 'Previous artwork', icon='left', action='art-prev'), text(44, 255, '1 / 6', '#989eac', 12, cls='art-index')]
    n += [button(option, 167, 244, 22, 'Next artwork', icon='right', action='art-next'), button(option, 207, 244, 80, 'CHOOSE'), button(option, 299, 244, 64, 'RETRY')]
    n += [text(418, 255, 'Click art: original ↗', '#989eac', 10)]
    n += [text(0, 307, 'ACROSS THE RACK', '#989eac', 10), text(0, 332, 'Player', '#989eac', 10)]
    for i, icon in enumerate(['previous', 'play', 'pause', 'stop', 'next']):
        n += [button(option, 66 + i * 28, 321, 22, icon.title(), 'active' if icon == 'play' else '', icon=icon, action='transport-' + icon)]
    n += [button(option, 219, 321, 76, 'SHUFFLE'), button(option, 303, 321, 76, 'REP OFF')]
    n += [text(0, 366, 'Playlist', '#989eac', 10)]
    for i, label in enumerate(['ADD', 'REM', 'SEL', 'MISC', 'LIST']):
        n += [button(option, 66 + i * 52, 355, 44, label)]
    n += [text(354, 366, 'EQ', '#989eac', 10), button(option, 380, 355, 64, 'IMPORT'), button(option, 452, 355, 64, 'EXPORT'), button(option, 524, 355, 68, 'SAVE AS')]
    n += [text(0, 407, 'STATES', '#989eac', 10)]
    for i, (state, label) in enumerate([('', 'Default'), ('hover', 'Hover'), ('pressed', 'Pressed'), ('focused', 'Focus'), ('disabled', 'Disabled')]):
        n += [button(option, i * 121, 420, 100 if i < 4 else 108, 'CHOOSE', state), text(i * 121 + 50, 456, label, '#989eac', 9, anchor='middle')]
    return ''.join(n) + '</svg>'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cover', type=Path)
    parser.add_argument('--reference', type=Path)
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    cover = uri(args.cover) if args.cover else None
    articles = []
    for option in OPTIONS:
        key, name, description, *_ = option
        svg = study(option, cover)
        (OUT / f'buttons-{key}.svg').write_text(svg)
        for scale in [1, 2]:
            subprocess.run(['rsvg-convert', '-w', str(592 * scale), '-h', str(464 * scale), str(OUT / f'buttons-{key}.svg'), '-o', str(OUT / f'buttons-{key}-{scale}x.png')], check=True)
        articles.append(f'<article id="option-{key}"><h2>{key.upper()} — {name}</h2><p>{description}</p><div class="study">{svg}</div><div class="links"><a href="buttons-{key}-1x.png">1× image ↗</a><a href="buttons-{key}-2x.png">2× image ↗</a><a href="buttons-{key}.svg">SVG ↗</a></div></article>')
    reference = f'<section class="reference"><div><h2>Current buttons</h2><p>29px high · Choose 98px wide · Retry 80px wide</p><p>The alternatives below use 22px height, 11px lettering, and 80px / 64px action widths. The Album panel stays the same size.</p><p>Start with <a href="#option-a">A</a> to judge size alone, or <a href="#option-b">B</a> for a quieter treatment.</p></div><img src="{uri(args.reference)}" alt="Current Album pane supplied for comparison" width="592" height="287"></section>' if args.reference else ''
    page = '''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>STAR/AMP — Button treatments</title><style>
    :root{color-scheme:dark;font-family:system-ui,sans-serif;background:#171820;color:#d7d9cf}*{box-sizing:border-box}body{margin:0}main{max-width:1328px;margin:auto;padding:32px}h1{font-size:32px;letter-spacing:-.8px;margin:0 0 12px}h2{font-size:17px;font-weight:600;margin:0 0 8px}p{color:#989eac;font-size:14px;line-height:1.6;margin:8px 0 16px}a{color:#a4d791;text-underline-offset:3px}nav{display:flex;gap:20px;flex-wrap:wrap;margin:24px 0 32px}.reference{display:grid;grid-template-columns:1fr 592px;gap:36px;border-top:1px solid #353946;border-bottom:1px solid #353946;padding:24px 0;margin-bottom:32px}.reference img{display:block;max-width:100%;height:auto}.grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:32px}article{padding:0 0 28px;border-bottom:1px solid #353946;scroll-margin:20px}.study{overflow:auto;background:#171820}.study>svg{display:block;width:592px;height:464px;max-width:none}.links{display:flex;gap:20px;font-size:12px;margin-top:14px}.note{margin:28px 0 0;font-size:12px}@media(max-width:1250px){main{max-width:656px}.grid,.reference{grid-template-columns:1fr}.reference{gap:12px}}@media(max-width:650px){main{padding:20px 12px}h1{font-size:26px}.reference img{width:592px;max-width:none}.reference{overflow:auto}.study{padding-bottom:8px}}
    article,.reference>div{min-width:0}@media(max-width:1250px){.grid,.reference{grid-template-columns:minmax(0,1fr)}}
    </style><main><h1>Button treatments</h1><p>Four alternatives using your Album pane and the same rack palette. Every alternative is shown at 1:1 size with matching Player, Playlist and EQ controls.</p><nav><a href="#option-a">A · Compact Classic</a><a href="#option-b">B · Quiet bevel</a><a href="#option-c">C · Thin outline</a><a href="#option-d">D · Minimal</a><a href="approved.html">Selected rack design ↗</a></nav>'''+reference+'<section class="grid">'+''.join(articles)+'''</section><p class="note">Design studies. Hover, press, or Tab through the controls; artwork arrows cycle the sample counter, and transport controls demonstrate their active state.</p></main><script>
    document.querySelectorAll('.button:not(.disabled)').forEach(button=>{
      const activate=()=>{const svg=button.closest('svg');const action=button.dataset.action;
        if(action.startsWith('art-')){const index=svg.querySelector('.art-index');let n=parseInt(index.textContent);n=((n-1+(action==='art-next'?1:5))%6)+1;index.textContent=n+' / 6'}
        else if(action.startsWith('transport-')){svg.querySelectorAll('[data-action^="transport-"]').forEach(b=>b.classList.remove('active'));button.classList.add('active')}
      };
      button.addEventListener('click',activate);
      button.addEventListener('keydown',event=>{if(event.key==='Enter'||event.key===' '){event.preventDefault();button.classList.add('pressed');activate()}});
      button.addEventListener('keyup',()=>button.classList.remove('pressed'));
      button.addEventListener('blur',()=>button.classList.remove('pressed'));
    });
    </script></html>'''
    (OUT / 'buttons.html').write_text(page)
    rows = []
    for i, option in enumerate(OPTIONS):
        key, name, *_ = option
        x, y = 24 + (i % 2) * 624, 70 + (i // 2) * 520
        rows += [text(x, y, f'{key.upper()} — {name}', size=15, bold=True)]
        rows += [f'<image x="{x}" y="{y+18}" width="592" height="464" href="{uri(OUT / f"buttons-{key}-1x.png")}"/>']
    overview = f'<svg xmlns="http://www.w3.org/2000/svg" width="1264" height="1090"><style>svg{{font-family:{FONT}}}</style>'+rect(0, 0, 1264, 1090, '#171820')+text(24, 28, 'STAR/AMP · 22px button treatments', size=18, bold=True)+''.join(rows)+'</svg>'
    (OUT / 'buttons-overview.svg').write_text(overview)
    subprocess.run(['rsvg-convert', str(OUT / 'buttons-overview.svg'), '-o', str(OUT / 'buttons-overview.png')], check=True)
    print(OUT / 'buttons.html')


if __name__ == '__main__':
    main()
