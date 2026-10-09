#!/usr/bin/env python3
"""Deterministic design studies; not screenshots of implemented interfaces.

Uses the installed terminal font. Requires rsvg-convert for PNG output.
Run from any directory: python3 scripts/design/treatments.py
"""
from pathlib import Path
from html import escape
import math
import subprocess

OUT = Path(__file__).resolve().parents[2] / 'docs/graphical/treatments'
OUT.mkdir(parents=True, exist_ok=True)
W, H = 1400, 1080
TRACKS = [
    ('Through the Water and the Waves', '04:58'),
    ('Turn of the Tide', '05:25'), ('Stranger in the Dark', '04:36'),
    ('Now and Forever', '04:12'), ('When Blue Turns to Gray', '05:31'),
    ('Take Me Home', '04:42'), ('The Longest Night', '05:18'),
    ('The Shore', '03:56'), ('Ocean of Dreams', '05:49'),
]
PALETTES = {
    'dark': dict(bg='#1e1e28', panel='#22222d', fg='#d8d5c5', muted='#93939f',
                 border='#454550', accent='#8ca9e3', select='#343b50', soft='#292c38'),
    'light': dict(bg='#eeeef2', panel='#f7f7fa', fg='#30313b', muted='#6d6e7b',
                  border='#c7c8d0', accent='#365f9c', select='#dce4f3', soft='#e6e9f0'),
}

class Drawing:
    def __init__(self, light=False):
        self.p = PALETTES['light' if light else 'dark']
        self.nodes = []
        self.rect(0, 0, W, H, 'bg')

    def color(self, c):
        return self.p.get(c, c)

    def rect(self, x, y, w, h, fill='panel', radius=9, stroke=None):
        self.nodes.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" '
                          f'rx="{radius}" fill="{self.color(fill)}" '
                          f'stroke="{self.color(stroke) if stroke else "none"}" stroke-width="2"/>')

    def text(self, x, y, text, c='fg', size=14, bold=False, anchor='start'):
        self.nodes.append(f'<text x="{x}" y="{y}" fill="{self.color(c)}" '
                          f'font-family="JetBrains Mono,monospace" font-size="{size}" '
                          f'font-weight="{600 if bold else 400}" text-anchor="{anchor}">{escape(text)}</text>')

    def line(self, x, y, xx, yy, c='border', width=1):
        self.nodes.append(f'<path d="M{x},{y} L{xx},{yy}" fill="none" stroke="{self.color(c)}" '
                          f'stroke-width="{width}" stroke-linecap="round"/>')

    def panel(self, x, y, w, h, name, actions='', focus=False, filled=True):
        self.rect(x, y, w, h, 'panel' if filled else 'bg', stroke='accent' if focus else 'border')
        self.text(x+16, y+25, name, 'accent', 13, True)
        self.text(x+w-16, y+25, actions, 'muted', 12, anchor='end')

    def bars(self, x, y, w, h, kind=0):
        count = 54
        for i in range(count):
            v = .15 + .7 * abs(math.sin(i*.085+1.3)) * (.7+.3*math.sin(i*.43))
            bh = max(3, int(h*v))
            xx = x+i*w/count
            if kind == 1:
                for k in range(0, bh, 7):
                    self.rect(xx, y+h-k-4, max(2, w/count-4), 4, 'accent', 1)
            else:
                self.rect(xx, y+h-bh, max(2, w/count-4), bh, 'accent', 2)
                self.line(xx, y+h-bh-5, xx+max(2,w/count-4), y+h-bh-5, 'muted')

    def transport(self, x, y, w):
        for i, name in enumerate(('prev','pause','stop','next')):
            bx=x+i*42
            self.rect(bx,y,32,30,'accent' if name=='pause' else 'soft',6)
            ink='bg' if name=='pause' else 'fg'
            if name=='pause':
                self.rect(bx+10,y+8,4,14,ink,1); self.rect(bx+18,y+8,4,14,ink,1)
            elif name=='stop': self.rect(bx+10,y+9,12,12,ink,1)
            else:
                a,b=(bx+20,bx+11) if name=='prev' else (bx+11,bx+20)
                self.nodes.append(f'<path d="M{a},{y+8} L{b},{y+15} L{a},{y+22} Z" fill="{self.color(ink)}"/>')
                self.line(b,y+8,b,y+22,ink,2)
        self.text(x+188,y+20,'SHUF   REP ALL   FADE','muted',12)
        self.text(x+w-190,y+20,'VOL','muted',12)
        self.line(x+w-150,y+15,x+w-44,y+15,'border',4)
        self.line(x+w-150,y+15,x+w-66,y+15,'accent',4)
        self.text(x+w-8,y+20,'80','fg',12,anchor='end')

    def seek(self,x,y,w):
        self.text(x,y+4,'01:09','muted',12)
        self.line(x+62,y,x+w-62,y,'border',3)
        self.line(x+62,y,x+62+(w-124)*.23,y,'accent',3)
        self.text(x+w,y+4,'04:58','muted',12,anchor='end')

    def player(self,x,y,w,h,kind=0):
        self.panel(x,y,w,h,'S T A R / A M P','bit-perfect · FLAC',True)
        self.text(x+18,y+85,'1:09','accent',40)
        self.bars(x+155,y+49,w-175,min(62,h-164),kind)
        self.text(x+18,y+h-88,'Terra Atlantica — Through the Water and the Waves',size=15,bold=True)
        self.text(x+18,y+h-66,'Oceans · 2025','muted',12)
        self.text(x+w-18,y+h-66,'1063 kbps · 44.1 kHz · 16-bit · stereo','muted',12,anchor='end')
        self.seek(x+18,y+h-48,w-36)
        self.transport(x+18,y+h-36,w-36)

    def cover(self,x,y,size):
        # Original geometric placeholder; no borrowed or invented album artwork.
        self.rect(x,y,size,size,'#24394c',12)
        clip = f'cover-{x}-{y}-{size}'
        self.nodes.append(f'<defs><clipPath id="{clip}"><rect x="{x}" y="{y}" width="{size}" height="{size}" rx="12"/></clipPath></defs><g clip-path="url(#{clip})">')
        for i in range(7):
            self.nodes.append(f'<path d="M{x},{y+size*(.45+i*.07)} Q{x+size*.35},{y+size*(.2+i*.07)} '
                              f'{x+size*.65},{y+size*(.5+i*.07)} T{x+size},{y+size*(.35+i*.07)}" '
                              f'fill="none" stroke="#7ca9bf" stroke-opacity="{.2+i*.08}" stroke-width="2"/>')
        self.text(x+size/2,y+size*.22,'O C E A N S','#d4e3ed',max(11,int(size*.075)),True,'middle')
        self.nodes.append('</g>')

    def album(self,x,y,w,h):
        self.panel(x,y,w,h,'ALBUM','settings   close')
        size=min(h-54,110)
        self.cover(x+16,y+42,size)
        self.text(x+size+32,y+65,'Oceans','accent',16,True)
        self.text(x+size+32,y+89,'Terra Atlantica · 2025',size=13)
        self.text(x+size+32,y+112,'11 tracks · 49:17 · FLAC','muted',12)
        self.text(x+size+32,y+136,'embedded 1/2   ‹   ›','muted',12)

    def activity(self,x,y,w,h):
        self.panel(x,y,w,h,'ACTIVITY','settings   close')
        self.text(x+16,y+51,'Last.fm ON · ListenBrainz OFF','muted',12)
        for i,t in enumerate(['Through the Water and the Waves','Turn of the Tide','Stranger in the Dark']):
            if 76+i*23 > h-12: break
            self.text(x+16,y+76+i*23,t,size=12)
            self.text(x+w-16,y+76+i*23,'playing' if not i else 'sent','accent' if not i else 'muted',12,anchor='end')

    def queue(self,x,y,w,h,compact=False):
        self.panel(x,y,w,h,'PLAYLIST · Oceans','sorting   gravity   settings   close')
        self.text(x+16,y+50,'11 tracks · 49:17','muted',12)
        self.text(x+w-16,y+50,'1 / 11','muted',12,anchor='end')
        row=27 if compact else 30
        for i,(name,dur) in enumerate(TRACKS):
            yy=y+75+i*row
            if yy>y+h-12: break
            if i==0:self.rect(x+8,yy-17,w-16,row-2,'select',5)
            self.text(x+18,yy,'▶' if i==0 else f'{i+1:02}', 'accent' if i==0 else 'muted',12)
            self.text(x+53,yy,name,size=13)
            self.text(x+w-18,yy,dur,'muted',12,anchor='end')

    def equalizer(self,x,y,w,h):
        self.panel(x,y,w,h,'EQUALIZER','settings   close')
        self.text(x+16,y+52,'ON   ‹ Reference ›','accent',12)
        for i,t in enumerate(['Peaking   80 Hz   +2.0 dB','Peaking  1.2 kHz  -1.5 dB','Shelf    8.0 kHz  +1.0 dB']):
            self.text(x+16,y+83+i*26,t,size=12)

    def footer(self,y):
        self.line(20,y-19,W-20,y-19)
        self.text(22,y,'? help','muted',12)
        self.text(W-22,y,'Playing · FLAC · bit-perfect · REP ALL · local','accent',12,anchor='end')

    def embed(self,kind):
        self.text(22,837,'MATCHING FOLD EMBED · complete now-playing', 'muted',12,True)
        self.player(20,852,1360,208,kind)

    def save(self,name):
        svg=OUT/f'{name}.svg'
        svg.write_text(f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">' + ''.join(self.nodes)+'</svg>')
        subprocess.run(['rsvg-convert','-o',str(svg.with_suffix('.png')),str(svg)],check=True)

def faithful(n,title,light=False):
    d=Drawing(light)
    d.text(22,28,f'{n:02} · {title}', 'fg',16,True)
    d.text(W-22,28,'FAITHFUL · design study', 'muted',12,anchor='end')
    d.player(20,46,1360,236,n%2)
    if n==1:
        d.album(20,294,1360,164)
        d.activity(20,470,1360,150)
        d.queue(20,632,1360,146,True)
    elif n==2:
        d.album(20,294,660,166)
        d.activity(692,294,688,166)
        d.queue(20,472,1360,306,True)
    else:
        d.album(20,294,440,172)
        d.equalizer(20,478,440,190)
        d.queue(472,294,908,374)
        d.activity(20,680,1360,98)
    d.footer(806)
    d.embed(n%2)
    d.save(f'{n:02}-'+['','stacked','balanced','studio-light'][n])

def desktop(n,title):
    d=Drawing()
    d.text(22,28,f'{n:02} · {title}',size=16,bold=True)
    d.text(W-22,28,'EXPERIMENTAL · design study','muted',12,anchor='end')
    if n==4:
        d.panel(20,46,210,660,'S T A R / A M P')
        for i,t in enumerate(['Now playing','Library','Files','Playlists','Activity','Equalizer','Settings']):
            if i==0:d.rect(30,88,190,32,'select',6)
            d.text(42,109+i*43,t,'accent' if i==0 else 'muted',13)
        d.panel(242,46,420,660,'NOW PLAYING','art   settings')
        d.cover(274,102,356)
        d.text(274,498,'Through the Water',size=20,bold=True)
        d.text(274,525,'and the Waves',size=20,bold=True)
        d.text(274,555,'Terra Atlantica · Oceans','muted',13)
        d.text(274,583,'FLAC · 44.1 kHz · 16-bit','muted',12)
        d.bars(274,608,356,65,1)
        d.queue(674,46,706,660)
    elif n==5:
        d.panel(20,46,1360,68,'S T A R / A M P','Library   Files   Playlists   Activity   Equalizer   Settings')
        d.panel(20,126,500,580,'NOW PLAYING','art   settings')
        d.cover(100,175,340)
        d.text(46,555,'Through the Water and the Waves',size=17,bold=True)
        d.text(46,584,'Terra Atlantica · Oceans · 2025','muted',13)
        d.bars(46,609,446,62)
        d.queue(532,126,848,580)
    else:
        d.panel(20,46,1360,68,'S T A R / A M P','Library   Files   Playlists   Activity   Equalizer   Settings')
        d.album(20,126,1360,172)
        d.queue(20,310,896,396)
        d.equalizer(928,310,452,190)
        d.activity(928,512,452,194)
    d.rect(20,718,1360,60,'panel',9,stroke='border')
    d.seek(38,729,1324)
    d.transport(38,743,1324)
    d.footer(806)
    d.embed(n%2)
    d.save(f'{n:02}-'+{4:'library-desk',5:'listening-room',6:'queue-workbench'}[n])

if __name__ == '__main__':
    faithful(1,'Stacked reference')
    faithful(2,'Balanced panels')
    faithful(3,'Studio panels · light theme',True)
    desktop(4,'Library desk')
    desktop(5,'Listening room')
    desktop(6,'Queue workbench')
    print(f'Wrote six SVG/PNG treatment pairs to {OUT}')
