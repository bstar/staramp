#!/usr/bin/env python3
"""Editable original skin masters and density-specific raster assets."""
from pathlib import Path
import subprocess, json
ROOT=Path(__file__).resolve().parents[2]
OUT=ROOT/'assets/skins/classic'
PALETTE=dict(panel='#303340',shadow='#101117',highlight='#7c8294',title='#242733',ink='#d7d9cf',accent='#a4d791',well='#101813',raised='#474d5d')
def export(name,body,size=24):
    svg=f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {size} {size}">{body}</svg>'
    p=OUT/'source'/f'{name}.svg';p.write_text(svg)
    for scale in [1,2]:
        subprocess.run(['rsvg-convert','-w',str(size*scale),'-h',str(size*scale),str(p),'-o',str(OUT/f'{scale}x'/f'{name}.png')],check=True)
def rect(x,y,w,h,color,r=0):return f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{color}"/>'
def line(x,y,xx,yy,color):return f'<path d="M{x} {y} H{xx}" fill="none" stroke="{color}"/>' if y==yy else f'<path d="M{x} {y} V{yy}" fill="none" stroke="{color}"/>'
for rigid in [False,True]:
    body=rect(0,0,24,24,PALETTE['shadow'],0 if rigid else 4)+rect(1,1,22,22,PALETTE['panel'],0 if rigid else 3)
    for a in [(3,3,21,3,'highlight'),(3,3,3,21,'highlight'),(3,21,21,21,'shadow'),(21,3,21,21,'shadow')]:body+=line(*a[:4],PALETTE[a[4]])
    export('panel-rigid' if rigid else 'panel',body)
export('well',rect(0,0,24,24,'#1d2027',2)+rect(1,1,22,22,PALETTE['well'],1)+line(1,1,23,1,'#080b0a')+line(1,23,23,23,'#606575'))
for state in ['normal','hover','pressed','disabled','focus','active']:
    pressed=state=='pressed'
    fill={'normal':'#474d5d','hover':'#596172','pressed':'#3a4050','disabled':'#373b48','focus':'#474d5d','active':'#6b7282'}[state]
    body=rect(0,0,24,24,'#12141b')+rect(1,1,22,22,fill)
    top,bottom=('#11131a','#9da2af') if pressed else ('#9da2af','#11131a')
    for coords in [(2,2,22,2),(2,2,2,22)]:body+=line(*coords,top)
    for coords in [(2,22,22,22),(22,2,22,22)]:body+=line(*coords,bottom)
    if state=='focus':body+='<rect x="4.5" y="4.5" width="15" height="15" fill="none" stroke="#a4d791" stroke-dasharray="1 1"/>'
    export('button-'+state,body)
# Quieter FOLD-directed chrome; preserve Classic as an alternative treatment.
export('panel-fold',rect(0,0,24,24,'#747986',6)+rect(2,2,20,20,'#20212a',4))
export('panel-fold-rigid',rect(0,0,24,24,'#747986')+rect(2,2,20,20,'#20212a'))
export('well-fold',rect(0,0,24,24,'#171820',4))
for state in ['normal','hover','pressed','disabled','focus','active']:
    fill={'normal':'#303340','hover':'#3d4250','pressed':'#444958','disabled':'#262832','focus':'#303340','active':'#444958'}[state]
    body=rect(0,0,24,24,fill,3)
    if state=='focus':body=rect(0,0,24,24,'#a4d791',3)+rect(2,2,20,20,fill,1)
    export('button-fold-'+state,body)
# Original 5x7 control lettering. User content uses shaped outline text.
patterns={
'A':['01110','10001','10001','11111','10001','10001','10001'],
'B':['11110','10001','10001','11110','10001','10001','11110'],
'C':['01111','10000','10000','10000','10000','10000','01111'],
'D':['11110','10001','10001','10001','10001','10001','11110'],
'E':['11111','10000','10000','11110','10000','10000','11111'],
'F':['11111','10000','10000','11110','10000','10000','10000'],
'G':['01111','10000','10000','10111','10001','10001','01111'],
'H':['10001','10001','10001','11111','10001','10001','10001'],
'I':['111','010','010','010','010','010','111'],
'J':['00111','00010','00010','00010','10010','10010','01100'],
'K':['10001','10010','10100','11000','10100','10010','10001'],
'L':['10000','10000','10000','10000','10000','10000','11111'],
'M':['10001','11011','10101','10101','10001','10001','10001'],
'N':['10001','11001','10101','10011','10001','10001','10001'],
'O':['01110','10001','10001','10001','10001','10001','01110'],
'P':['11110','10001','10001','11110','10000','10000','10000'],
'Q':['01110','10001','10001','10001','10101','10010','01101'],
'R':['11110','10001','10001','11110','10100','10010','10001'],
'S':['01111','10000','10000','01110','00001','00001','11110'],
'T':['11111','00100','00100','00100','00100','00100','00100'],
'U':['10001','10001','10001','10001','10001','10001','01110'],
'V':['10001','10001','10001','10001','10001','01010','00100'],
'W':['10001','10001','10001','10101','10101','10101','01010'],
'X':['10001','10001','01010','00100','01010','10001','10001'],
'Y':['10001','10001','01010','00100','00100','00100','00100'],
'Z':['11111','00001','00010','00100','01000','10000','11111'],
' ':['000']*7,'/':['00001','00001','00010','00100','01000','10000','10000'],
'-':['00000','00000','00000','11111','00000','00000','00000'],
}
body='';glyphs={};x=0
for ch,rows in patterns.items():
    width=len(rows[0]);glyphs[ch]={'rect':dict(x=x,y=0,width=width,height=7),'advance':width+1}
    for y,row in enumerate(rows):
        for xx,v in enumerate(row):
            if v=='1':body+=rect(x+xx,y,1,1,'#ffffff')
    x+=width+1
p=OUT/'source/control-font.svg';p.write_text(f'<svg xmlns="http://www.w3.org/2000/svg" width="{x}" height="7">{body}</svg>')
subprocess.run(['rsvg-convert',str(p),'-o',str(OUT/'1x/control-font.png')],check=True)
(OUT/'control-font.json').write_text(json.dumps(dict(height=7,baseline=7,glyphs=glyphs),indent=2)+'\n')
(OUT/'manifest.json').write_text(json.dumps(dict(version=1,author='STAR/AMP',license='MIT',palette=PALETTE,insets=dict(panel=[6,6,6,6],well=[3,3,3,3],button=[4,4,4,4]),densities=[1,2]),indent=2)+'\n')
