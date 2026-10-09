#!/usr/bin/env python3
"""Winamp Classic-directed studies, using editable SVG and the terminal font."""
from pathlib import Path
import math
import subprocess
from treatments import Drawing, TRACKS

OUT=Path(__file__).resolve().parents[2]/'docs/graphical/treatments'

class Classic(Drawing):
    def __init__(self, warm=False, roundness=4):
        super().__init__()
        self.r=roundness
        self.square_transport=False
        self.p.update(bg='#171820',panel='#303340',border='#747986',fg='#d7d9cf',
                      muted='#989eac',accent='#a4d791' if not warm else '#dfba78',
                      select='#303f3d' if not warm else '#474036',soft='#444958')
        self.rect(0,0,1400,1080,'bg',0)

    def window(self,x,y,w,h,title,extra=''):
        self.rect(x,y,w,h,'panel',self.r,stroke='#101117')
        self.line(x+3,y+3,x+w-3,y+3,'#7c8294')
        self.line(x+3,y+3,x+3,y+h-3,'#7c8294')
        self.line(x+3,y+h-3,x+w-3,y+h-3,'#11131a')
        self.line(x+w-3,y+3,x+w-3,y+h-3,'#11131a')
        self.rect(x+7,y+7,w-14,24,'#242733',2)
        self.text(x+18,y+24,title,'#bbc6d5',12,True)
        self.text(x+w-18,y+24,extra or '−  ×','#a5aaba',12,anchor='end')

    def well(self,x,y,w,h):
        self.rect(x,y,w,h,'#101813',2,stroke='#1d2027')
        self.line(x+1,y+1,x+w-1,y+1,'#080b0a')
        self.line(x+1,y+h,x+w-1,y+h,'#606575')

    def digit(self,x,y,value,scale=1):
        masks={'0':'abcedf','1':'bc','2':'abged','3':'abgcd','4':'fgbc','5':'afgcd','6':'afgecd','7':'abc','8':'abcdefg','9':'abfgcd'}
        coords={'a':(3,0,17,3),'g':(3,23,17,3),'d':(3,46,17,3),
                'f':(0,3,3,20),'b':(20,3,3,20),'e':(0,26,3,20),'c':(20,26,3,20)}
        for char in value:
            if char==':':
                for yy in (15,33):self.rect(x+3*scale,y+yy*scale,3*scale,3*scale,'accent',0)
                x+=13*scale
                continue
            for seg,(xx,yy,w,h) in coords.items():
                self.rect(x+xx*scale,y+yy*scale,w*scale,h*scale,'accent' if seg in masks[char] else '#203226',0)
            x+=31*scale

    def leds(self,x,y,w,h):
        for i in range(56):
            count=int(3+12*abs(math.sin(i*.11+1))*(.7+.3*math.cos(i*.37)))
            for j in range(16):
                self.rect(x+i*w/56,y+h-j*h/16-2,w/56-3,2,
                          'accent' if j<count else '#1c2a20',0)

    def button(self,x,y,w,label,on=False,square=False):
        self.rect(x,y,w,29,'#6b7282' if on else '#474d5d',0 if square else 2,stroke='#12141b')
        self.line(x+2,y+2,x+w-2,y+2,'#9da2af')
        self.line(x+2,y+2,x+2,y+27,'#9da2af')
        ink='#10161a' if on else '#e0e2d5'
        cx,cy=x+w/2,y+14.5
        if label in ['|◀','▶','Ⅱ','■','▶|']:
            # Draw centered artwork rather than mixed font glyphs: the triangle
            # and ASCII stop bar have different bearings in terminal fonts.
            if label in ['|◀','▶|']:
                direction=-1 if label=='|◀' else 1
                self.nodes.append(f'<path d="M{cx-6*direction},{cy-6} L{cx+2*direction},{cy} L{cx-6*direction},{cy+6} Z" fill="{ink}"/>')
                self.rect(cx+4 if direction==1 else cx-6,cy-6,2,12,ink,0)
            elif label=='▶':
                self.nodes.append(f'<path d="M{cx-4},{cy-6} L{cx+5},{cy} L{cx-4},{cy+6} Z" fill="{ink}"/>')
            elif label=='Ⅱ':
                self.rect(cx-4.5,cy-6,3,12,ink,0)
                self.rect(cx+1.5,cy-6,3,12,ink,0)
            else:
                self.rect(cx-5,cy-5,10,10,ink,0)
        else:
            self.text(cx,y+20,label,ink,14,True,'middle')

    def player(self,x,y,w,h=230):
        self.window(x,y,w,h,'S T A R / A M P','bit-perfect   −  ×')
        self.well(x+14,y+43,w-28,110)
        self.digit(x+28,y+58,'1:09',1.1)
        self.text(x+28,y+138,'PLAY · STEREO','accent',11)
        self.leds(x+190,y+52,w-220,48)
        self.text(x+190,y+122,'Terra Atlantica — Through the Water and the Waves','accent',13)
        self.text(x+190,y+141,'Oceans · FLAC · 1063 kbps · 44.1 kHz · 16-bit','muted',11)
        self.seek(x+16,y+169,w-32)
        for i,label in enumerate(['|◀','▶','Ⅱ','■','▶|']):
            size,pitch=(29,35) if self.square_transport else (36,41)
            self.button(x+16+i*pitch,y+187,size,label,i==1,square=self.square_transport)
        self.button(x+237,y+187,100,'SHUFFLE')
        self.button(x+346,y+187,100,'REP ALL',True)
        self.text(x+w-220,y+207,'VOL','muted',11)
        self.line(x+w-180,y+202,x+w-50,y+202,'#151920',6)
        self.line(x+w-180,y+202,x+w-76,y+202,'accent',4)
        self.text(x+w-16,y+207,'80','fg',11,anchor='end')

    def queue(self,x,y,w,h):
        self.window(x,y,w,h,'PLAYLIST · OCEANS','sorting   gravity   settings   ×')
        self.well(x+10,y+38,w-20,h-78)
        for i,(name,duration) in enumerate(TRACKS):
            yy=y+59+i*25
            if yy>y+h-48:break
            if i==0:self.rect(x+14,yy-15,w-28,23,'select',1)
            self.text(x+22,yy,f'{i+1:02}.','accent',12)
            self.text(x+58,yy,name,'accent' if i==0 else 'fg',12)
            self.text(x+w-28,yy,duration,'muted',12,anchor='end')
        for i,label in enumerate(['ADD','REM','SEL','MISC','LIST']):
            self.button(x+14+i*63,y+h-35,57,label)
        self.text(x+w-18,y+h-14,'11 tracks · 49:17','muted',11,anchor='end')

    def eq(self,x,y,w,h=174):
        self.window(x,y,w,h,'EQUALIZER','settings   ×')
        self.button(x+14,y+40,48,'ON',True)
        self.text(x+76,y+59,'Reference · parametric','muted',11)
        self.well(x+14,y+77,w-28,h-91)
        for i in range(10):
            xx=x+30+i*(w-60)/10
            self.line(xx,y+90,xx,y+h-26,'#667064',2)
            yy=y+112+math.sin(i*.9)*10
            self.rect(xx-7,yy,14,7,'#929ba0',1)
        self.text(x+22,y+h-14,'80 Hz  +2.0 dB · Q 0.7','accent',10)

    def parametric(self,x,y,w,h=294):
        self.window(x,y,w,h,'PARAMETRIC EQUALIZER','settings   ×')
        self.button(x+14,y+40,48,'ON',True)
        self.well(x+76,y+40,420,29)
        self.text(x+88,y+60,'Profile: Headphone reference  ▾','accent',12)
        self.text(x+520,y+60,'Preamp  −4.0 dB','muted',12)
        self.button(x+w-294,y+40,82,'IMPORT')
        self.button(x+w-202,y+40,82,'EXPORT')
        self.button(x+w-110,y+40,96,'SAVE AS')
        gx,gy,gw,gh=x+48,y+86,w-410,132
        self.well(gx-26,gy-8,gw+38,gh+35)
        for db in [-12,-6,0,6,12]:
            yy=gy+gh*(12-db)/24
            self.line(gx,yy,gx+gw,yy,'#364438' if db else '#677e64')
            self.text(gx-8,yy+4,f'{db:+}','muted',10,anchor='end')
        for hz,label in [(20,'20'),(50,'50'),(100,'100'),(200,'200'),(500,'500'),(1000,'1k'),(2000,'2k'),(5000,'5k'),(10000,'10k'),(20000,'20k')]:
            xx=gx+gw*math.log10(hz/20)/3
            self.line(xx,gy,xx,gy+gh,'#26332a')
            self.text(xx,gy+gh+17,label,'muted',10,anchor='middle')
        def gain(t):
            return -4.0+2.6/(1+math.exp((t-.55)*8))-3.8*math.exp(-((t-1.45)/.23)**2)+1.4/(1+math.exp(-(t-2.5)*9))
        points=[]
        for i in range(241):
            t=3*i/240
            points.append((gx+gw*i/240,gy+gh*(12-gain(t))/24))
        path=' '.join(f'{"M" if i==0 else "L"}{xx:.2f},{yy:.2f}' for i,(xx,yy) in enumerate(points))
        self.nodes.append(f'<path d="{path}" fill="none" stroke="{self.color("accent")}" stroke-width="2.5"/>')
        for i,t in enumerate([.55,1.45,2.5],1):
            xx,yy=gx+gw*t/3,gy+gh*(12-gain(t))/24
            self.nodes.append(f'<circle cx="{xx}" cy="{yy}" r="8" fill="{self.color("accent")}" stroke="#d9e7cb" stroke-width="1"/>')
            self.text(xx,yy+4,str(i),'#101813',10,True,'middle')
        ex=x+w-328
        self.text(ex,y+100,'BAND 2 · PEAKING','accent',13,True)
        for i,(key,value) in enumerate([('Frequency','560 Hz'),('Gain','−3.8 dB'),('Q','1.40'),('Channels','L + R')]):
            yy=y+127+i*24
            self.text(ex,yy,key,'muted',12)
            self.well(ex+113,yy-16,188,22)
            self.text(ex+125,yy,value,'accent',12)
        self.button(ex,y+218,91,'BYPASS')
        self.button(ex+101,y+218,91,'DELETE')
        self.text(x+18,y+h-35,'1 Low shelf · 70 Hz · +2.6 dB     [2 Peaking · 560 Hz · −3.8 dB]     3 High shelf · 6.3 kHz · +1.4 dB','fg',11)
        self.text(x+18,y+h-14,'+ Add filter     ‹ Previous band   Next band ›','accent',11)
        self.text(x+w-18,y+h-14,'Drag: frequency / gain · wheel: Q · exact values →','muted',11,anchor='end')

    def info(self,x,y,w,h,activity=False):
        self.window(x,y,w,h,'ACTIVITY' if activity else 'ALBUM','settings   ×')
        if activity:
            for i,name in enumerate(['Last.fm ON · ListenBrainz OFF','Through the Water and the Waves · playing','Turn of the Tide · sent']):
                if 55+i*23<h-10:self.text(x+16,y+55+i*23,name,'muted' if i==0 else 'fg',11)
        else:
            self.cover(x+14,y+41,min(90,h-52))
            for i,name in enumerate(['Oceans','Terra Atlantica · 2025','11 tracks · 49:17 · FLAC','embedded 1/2  ‹  ›']):
                self.text(x+118,y+59+i*21,name,'accent' if i==0 else 'muted',12)

    def album_detail(self,x,y,w,h=292):
        self.window(x,y,w,h,'ALBUM','artist ↗   album ↗   settings   ×')
        self.cover(x+16,y+46,180)
        self.text(x+214,y+71,'Oceans','accent',22,True)
        self.text(x+214,y+96,'Terra Atlantica','fg',15)
        self.text(x+214,y+123,'2025  ·  11 tracks  ·  49:17','muted',12)
        self.text(x+214,y+148,'FLAC  ·  44.1 kHz  ·  16-bit','fg',12)
        self.text(x+214,y+172,'GENRE   Symphonic power metal','muted',11)
        self.line(x+214,y+185,x+w-18,y+185,'#454b58')
        self.text(x+214,y+207,'ARTWORK  Embedded front cover','accent',11)
        self.text(x+214,y+228,'Oceans/01 Through the Water…flac','muted',11)
        self.button(x+16,y+242,29,'|◀',square=True)
        self.text(x+106,y+262,'1 / 2','muted',12,anchor='middle')
        self.button(x+167,y+242,29,'▶|',square=True)
        self.button(x+214,y+242,99,'CHOOSE')
        self.button(x+323,y+242,81,'RETRY')
        self.text(x+w-18,y+262,'Click art: original ↗','muted',10,anchor='end')

    def activity_detail(self,x,y,w,h=292):
        self.window(x,y,w,h,'ACTIVITY','services   settings   ×')
        for offset,width,title,sub,color in [
            (14,210,'● Last.fm','Enabled · bstar','accent'),
            (232,220,'ListenBrainz','Not connected · Set up','muted'),
            (460,w-474,'Local history','Recording','accent'),
        ]:
            self.well(x+offset,y+42,width,43)
            self.text(x+offset+10,y+59,title,color,11,True)
            self.text(x+offset+10,y+76,sub,'muted',10)
        self.text(x+16,y+106,'RECENT LISTENS','muted',10,True)
        self.text(x+w-16,y+106,'DELIVERY','muted',10,True,anchor='end')
        rows=[('Through the Water and the Waves','01:09 listened · playing','PLAYING','accent'),
              ('Turn of the Tide','05:25 listened · completed','SENT','muted'),
              ('Stranger in the Dark','04:36 listened · completed','QUEUED','#d7b579'),
              ('Now and Forever','04:12 listened · retry available','ERROR','#e09885')]
        for i,(title,detail,status,color) in enumerate(rows):
            yy=y+126+i*34
            if not i:self.rect(x+10,yy-15,w-20,33,'select',1)
            self.text(x+16,yy,title,'fg',12)
            self.text(x+16,yy+14,'Terra Atlantica · '+detail,'muted',10)
            self.text(x+w-16,yy+2,status,color,10,True,anchor='end')
            if i:self.line(x+14,yy+20,x+w-14,yy+20,'#454b58')
        self.text(x+16,y+h-17,'2 pending · 1 needs attention','muted',11)
        self.button(x+w-156,y+h-35,140,'RETRY FAILED')

def render(n,title,layout,warm=False,rounded=False):
    d=Classic(warm,9 if rounded else 4)
    d.square_transport=n==1
    d.rect(0,0,1400,1130,'bg',0)
    d.text(24,28,f'{n:02} · {title}',size=17,bold=True)
    d.text(1376,28,'WINAMP CLASSIC · design study','muted',12,anchor='end')
    if layout==0:
        d.player(24,48,1352)
        if n==1:
            d.parametric(24,286,1352)
            d.album_detail(24,588,660)
            d.activity_detail(692,588,684)
            d.queue(24,888,1352,260)
        else:
            d.eq(24,286,1352,164)
            d.info(24,458,660,144)
            d.info(692,458,684,144,True)
            d.queue(24,610,1352,206)
    elif layout==1:
        d.player(24,48,1352)
        d.eq(24,286,468,174)
        d.info(24,468,468,160)
        d.info(24,636,468,180,True)
        d.queue(500,286,876,530)
    else:
        d.window(24,48,232,768,'S T A R / A M P')
        for i,name in enumerate(['Now playing','Library','Files','Playlists','Activity','Equalizer','Settings']):
            if not i:d.rect(34,89,212,28,'select',2)
            d.text(44,109+i*34,name,'accent' if not i else 'muted',13)
        d.player(264,48,1112)
        d.info(264,286,1112,144)
        d.queue(264,438,1112,378)
    extra=332 if n==1 else 0
    d.text(24,842+extra,'? help','muted',11)
    d.text(1376,842+extra,'Playing · REP ALL · local · bit-perfect','accent',11,anchor='end')
    d.text(24,876+extra,'MATCHING FOLD EMBED · AMP-owned compact player','muted',12)
    d.player(24,888+extra,1352,230)
    svg=OUT/f'winamp-{n:02}.svg'
    height=1130+extra
    svg.write_text(f'<svg xmlns="http://www.w3.org/2000/svg" width="1400" height="{height}" viewBox="0 0 1400 {height}"><rect width="1400" height="{height}" fill="{d.p["bg"]}"/>'+''.join(d.nodes)+'</svg>')
    subprocess.run(['rsvg-convert','-o',str(svg.with_suffix('.png')),str(svg)],check=True)

if __name__=='__main__':
    for args in [(1,'Classic stacked rack',0),(2,'Classic docked windows',1),(3,'Classic rounded dock',1,False,True),
                 (4,'Classic media library',2),(5,'Classic amber rack',0,True),(6,'Classic amber library',2,True,True)]:render(*args)
    print('Rendered six Winamp Classic studies')
