#!/usr/bin/env python3
"""Border-only HTML studies over an unchanged native player capture."""
import base64
import html
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'docs/graphical/treatments'
OPTIONS = [
    ('a', 'Previous bevel', 'The earlier edge: a slim highlight and a quiet dark return. A familiar starting point.'),
    ('b', 'Fine bevel', 'One crisp light edge and one crisp dark edge. Balanced relief with less visual weight.'),
    ('c', 'Soft contour', 'A continuous silver-gray rim with softer corners. Depth without the hard stair-step.'),
    ('d', 'Thin outline', 'One muted silver line. The blue-gray panel gradient supplies the depth.'),
]


def rect(x, y, w, h, fill, radius=0):
    return f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{radius}" fill="{fill}"/>'


def edge(x, y, w, h, light, dark):
    return (f'<path d="M{x} {h-y}V{y}H{w-x}" fill="none" stroke="{light}"/>'
            f'<path d="M{x} {h-y}H{w-x}V{y}" fill="none" stroke="{dark}"/>')


def panel(key, prefix, width, height, image, source_x=0, source_y=0):
    grad = f'{prefix}-body'
    clip = f'{prefix}-content'
    defs = f'''<defs><linearGradient id="{grad}" x1="0" y1="0" x2="0" y2="1">
    <stop stop-color="#212132"/><stop offset=".48" stop-color="#35354d"/>
    <stop offset="1" stop-color="#202030"/></linearGradient>
    <clipPath id="{clip}"><rect x="6" y="6" width="{width-12}" height="{height-12}"/></clipPath></defs>'''
    if key == 'current':
        return f'<svg width="{width}" height="{height}" viewBox="0 0 {width} {height}"><image x="{-source_x}" y="{-source_y}" width="1400" height="1200" href="{image}"/></svg>'
    if key == 'a':
        frame = rect(0, 0, width, height, '#10101b', 4)
        frame += rect(1, 1, width-2, height-2, '#28283b', 3)
        frame += edge(3, 3, width, height, '#7e8299', '#10101b')
        frame += rect(4, 4, width-8, height-8, f'url(#{grad})')
    elif key == 'b':
        frame = rect(0, 0, width, height, '#11111d', 4)
        frame += rect(1, 1, width-2, height-2, f'url(#{grad})', 3)
        frame += edge(2.5, 2.5, width, height, '#6c7088', '#080811')
    elif key == 'c':
        rim = f'{prefix}-rim'
        defs += f'''<defs><linearGradient id="{rim}" x1="0" y1="0" x2="0" y2="1">
        <stop stop-color="#7b8097"/><stop offset=".25" stop-color="#575b74"/>
        <stop offset=".65" stop-color="#32354e"/><stop offset="1" stop-color="#11111f"/>
        </linearGradient></defs>'''
        frame = rect(0, 0, width, height, '#10101c', 7)
        frame += rect(1, 1, width-2, height-2, f'url(#{rim})', 6)
        frame += rect(3, 3, width-6, height-6, f'url(#{grad})', 4)
    else:
        frame = rect(0, 0, width, height, '#60657c', 4)
        frame += rect(1, 1, width-2, height-2, f'url(#{grad})', 3)
    content = f'<image x="{-source_x}" y="{-source_y}" width="1400" height="1200" href="{image}" clip-path="url(#{clip})"/>'
    return defs + frame + content


def study(key, context, image, standalone=False):
    height = 230 if context == 'player' else 320
    prefix = f'border-{key}-{context}'
    if context == 'player':
        content = panel(key, prefix, 1400, height, image)
    else:
        content = panel(key, prefix+'-album', 696, height, image, 0, 540)
        content += f'<g transform="translate(704 0)">{panel(key, prefix+"-activity", 696, height, image, 704, 540)}</g>'
    return (f'<svg xmlns="http://www.w3.org/2000/svg" class="scene scene-{context}" width="1400" height="{height}" '
            f'viewBox="0 0 1400 {height}" role="img" aria-label="{html.escape(key.upper())} border treatment, {context}">'
            + (content if standalone else f'<g id="drawing-{key}-{context}">{content}</g>') + '</svg>')


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    capture = OUT / 'borders-native-rack.png'
    if not capture.exists():
        capture.write_bytes((OUT/'winamp-silver-full.png').read_bytes())
    image = 'data:image/png;base64,' + base64.b64encode(capture.read_bytes()).decode()
    cards = []
    for key, name, description in OPTIONS:
        for context in ('player', 'panes'):
            source = OUT / f'borders-{key}-{context}.svg'
            source.write_text(study(key, context, image, standalone=True) + '\n')
            for density in (1, 2):
                height = 230 if context == 'player' else 320
                subprocess.run(['rsvg-convert', '-w', str(1400*density), '-h', str(height*density),
                                str(source), '-o', str(OUT/f'borders-{key}-{context}-{density}x.png')], check=True)
        cards.append(f'''<article id="option-{key}" data-option="{key}">
        <div class="card-heading"><div><h2><span>{key.upper()}</span> {name}</h2><p>{description}</p></div>
        <button class="focus-option" data-focus="{key}" aria-label="Focus option {key.upper()}">Focus {key.upper()}</button></div>
        <div class="stage">{study(key,'player',capture.name)}{study(key,'panes',capture.name)}</div>
        <details open><summary>Corner detail · 3×</summary><div class="corners">
        <figure><figcaption>Light edge · top left</figcaption><svg class="loupe top" width="180" height="126" viewBox="0 0 60 42" aria-label="Top left corner enlarged three times"><use href="#drawing-{key}-player"/></svg></figure>
        <figure><figcaption>Dark edge · bottom right</figcaption><svg class="loupe bottom" width="180" height="126" viewBox="1340 188 60 42" aria-label="Bottom right corner enlarged three times"><use href="#drawing-{key}-player"/></svg></figure>
        <p class="detail-note">Same content and panel shading.<br>Only the outer frame changes.</p></div></details>
        <div class="exports"><span>Export player:</span><a data-export="svg" href="borders-{key}-player.svg">SVG</a><a data-export="1x" href="borders-{key}-player-1x.png">PNG 1×</a><a data-export="2x" href="borders-{key}-player-2x.png">PNG 2×</a><a class="permalink" href="#option-{key}">Link to {key.upper()}</a></div>
        </article>''')
    nav = ''.join(f'<button data-focus="{key}" aria-pressed="false">{key.upper()} · {name}</button>' for key,name,_ in OPTIONS)
    page = '''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
    <title>STAR/AMP · Border treatments</title><style>
    :root{color-scheme:dark;font:14px/1.6 system-ui,sans-serif;color:#cfd8e1;background:#1c1c27}
    *{box-sizing:border-box}body{margin:0}main{max-width:1480px;margin:auto;padding:36px 28px 50px}
    .eyebrow{color:#a4d791;font-size:11px;letter-spacing:2px}h1{font-size:32px;letter-spacing:-.8px;line-height:1.25;margin:10px 0}
    p{color:#9aa1b7;margin:6px 0 16px}a{color:#a4d791;text-underline-offset:3px}button{font:inherit;cursor:pointer}
    button,select{color:#cfd8e1;background:#28283b;border:1px solid #50546a;border-radius:4px;padding:7px 12px}
    button:hover{background:#34344a}button[aria-pressed="true"]{border-color:#a4d791;color:#a4d791;background:#303344}
    button:focus-visible,select:focus-visible,a:focus-visible,summary:focus-visible{outline:2px solid #a4d791;outline-offset:3px}
    nav{display:flex;gap:8px;flex-wrap:wrap;margin:22px 0 14px}.controls{display:flex;align-items:center;gap:22px;flex-wrap:wrap;padding-bottom:24px;border-bottom:1px solid #3b3d50}
    label{display:flex;align-items:center;gap:8px;color:#aeb5c7}input{accent-color:#a4d791}select{font:inherit}
    article,.reference{margin:28px 0 40px;padding-bottom:24px;border-bottom:1px solid #3b3d50;scroll-margin:18px}
    .card-heading{display:flex;justify-content:space-between;gap:20px;align-items:start}h2{font-size:18px;margin:0;line-height:1.5}
    h2 span{color:#a4d791;margin-right:8px}.card-heading p{font-size:13px;margin:6px 0 18px}.focus-option{white-space:nowrap}
    .stage{overflow-x:auto;background:#1c1c27}.scene{display:block;width:100%;max-width:1400px;height:auto}.scene-panes{display:none}
    body.panes .scene-player{display:none}body.panes .scene-panes{display:block}body.native .stage>.scene{width:1400px;max-width:none}
    details{margin-top:16px}summary{color:#b5bfd0;cursor:pointer;font-size:12px;width:fit-content}.corners{display:flex;align-items:center;gap:24px;flex-wrap:wrap;margin-top:10px}
    figure{margin:0}figcaption{color:#9aa1b7;font-size:10px;margin-bottom:7px}.loupe{display:block;image-rendering:pixelated;background:#1c1c27;border:1px solid #3b3d50}
    .detail-note{font-size:12px;margin:0}.exports{display:flex;gap:18px;flex-wrap:wrap;font-size:12px;margin-top:16px}.exports span{color:#9aa1b7}.permalink{margin-left:auto}
    .reference{border:0;border-bottom:1px solid #3b3d50;padding:0 0 24px}.reference p{font-size:12px}.note{font-size:12px;margin-top:32px}
    [hidden]{display:none!important}@media(max-width:650px){main{padding:22px 12px}h1{font-size:27px}.card-heading{gap:8px}.card-heading p{font-size:12px}.focus-option{padding:5px 8px}.controls{gap:12px}.corners{gap:12px}.detail-note{display:none}.permalink{margin-left:0}}
    </style><main><div class="eyebrow">STAR/AMP · FRAME STUDIES</div><h1>Less weight. Clearer relief.</h1>
    <p>Four border treatments around the same native player content. Muted green, silver playback buttons, and the blue-gray panel gradient stay consistent.</p>
    <nav aria-label="Treatment filter"><button data-focus="all" aria-pressed="true">All options</button>''' + nav + '''</nav>
    <div class="controls"><label>Context <select id="context"><option value="player">Player</option><option value="panes">Album + Activity</option></select></label>
    <label><input id="native" type="checkbox"> Actual size · 1:1</label><label><input id="compare" type="checkbox"> Show latest version for comparison</label></div>
    <section class="reference" id="reference" hidden><h2>Latest version · comparison</h2><p>The heavier two-step bevel currently in the app.</p><div class="stage">''' + study('current','player',capture.name) + study('current','panes',capture.name) + '''</div></section>
    <div id="options">''' + ''.join(cards) + '''</div>
    <p class="note">HTML design studies over a native renderer capture. These alternatives have not been applied to the app.<br>
    <a href="winamp-silver.html">Current native rack</a> · <a href="panes.html">Album + Activity layout studies</a> · <a href="buttons.html">Button studies</a></p></main>
    <script>
    const cards=[...document.querySelectorAll('article[data-option]')];
    function focusOption(key,updateHash=true){
      if(!['all','a','b','c','d'].includes(key))key='all';
      cards.forEach(card=>card.hidden=key!=='all'&&card.dataset.option!==key);
      document.querySelectorAll('nav [data-focus]').forEach(button=>button.setAttribute('aria-pressed',String(button.dataset.focus===key)));
      if(updateHash)history.replaceState(null,'',key==='all'?'#all':'#option-'+key);
    }
    document.querySelectorAll('[data-focus]').forEach(button=>button.addEventListener('click',()=>focusOption(button.dataset.focus)));
    document.getElementById('native').addEventListener('change',event=>document.body.classList.toggle('native',event.target.checked));
    document.getElementById('compare').addEventListener('change',event=>document.getElementById('reference').hidden=!event.target.checked);
    document.getElementById('context').addEventListener('change',event=>{
      const context=event.target.value;document.body.classList.toggle('panes',context==='panes');
      cards.forEach(card=>{
        const key=card.dataset.option;
        card.querySelectorAll('.loupe use').forEach(use=>use.setAttribute('href','#drawing-'+key+'-'+context));
        card.querySelector('.loupe.bottom').setAttribute('viewBox','1340 '+(context==='panes'?278:188)+' 60 42');
        card.querySelector('.exports span').textContent='Export '+(context==='panes'?'panes':'player')+':';
        card.querySelectorAll('[data-export]').forEach(link=>{const kind=link.dataset.export;link.href='borders-'+key+'-'+context+(kind==='svg'?'.svg':'-'+kind+'.png');});
      });
    });
    function fromHash(){focusOption(location.hash.startsWith('#option-')?location.hash.slice(8):'all',false);}
    document.querySelectorAll('.permalink').forEach(link=>link.addEventListener('click',()=>focusOption(link.hash.slice(8),false)));
    addEventListener('hashchange',fromHash);fromHash();
    </script></html>'''
    (OUT/'borders.html').write_text(page + '\n')
    print('Wrote borders.html, four border studies with player/pane SVG and 1x/2x exports.')


if __name__ == '__main__':
    main()
