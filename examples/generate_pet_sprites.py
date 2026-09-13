#!/usr/bin/env python3
"""Author Flere's embedded pixel sprites. Pillow is only an optional art-authoring tool.
Normal offline Rust builds consume the checked-in palette/RLE asset directly.
Run: python3 examples/generate_pet_sprites.py --out src/assets/mascots.rle
"""
from pathlib import Path
from PIL import Image,ImageDraw,ImageFont
import math,json
ROOT=Path(__file__).parent
BG='#080c14';EDGE='#141c32';INK='#060b16';CYAN='#53eef0';MAGENTA='#f778cd';GOLD='#f6c866';LIGHT='#ffedaa';SHADE='#c88d45'
def canvas():
 im=Image.new('RGBA',(64,64));return im,ImageDraw.Draw(im)
def duck(t=0,action='idle'):
 im,d=canvas();moving=action=='walk';sleep=action=='sleep';ponder=action=='ponder'
 bob=round(math.sin(t*math.tau*2)) if moving else 0
 # Short feet under a round body. All art is integer-aligned and palette-limited.
 foot=round(math.sin(t*math.tau)*2) if moving else 0
 d.rounded_rectangle((22-foot,50,32-foot,54),2,fill=INK);d.rectangle((24-foot,51,30-foot,52),fill='#eb964c')
 d.rounded_rectangle((35+foot,50,45+foot,54),2,fill=INK);d.rectangle((37+foot,51,43+foot,52),fill='#eb964c')
 # Magenta utility pack and tiny tail.
 d.rounded_rectangle((16,32+bob,25,46+bob),3,fill=INK);d.rectangle((17,35+bob,20,43+bob),fill='#925690');d.rectangle((18,35+bob,19,38+bob),fill=MAGENTA)
 d.polygon([(20,39+bob),(14,35+bob),(15,44+bob),(23,47+bob)],fill=INK)
 d.polygon([(20,40+bob),(17,38+bob),(18,43+bob),(22,45+bob)],fill=GOLD)
 d.ellipse((19,28+bob,46,51+bob),fill=INK);d.ellipse((21,30+bob,44,49+bob),fill=SHADE)
 d.ellipse((22,29+bob,43,46+bob),fill=GOLD)
 # Dark little vest, seam and brass clasp.
 d.rounded_rectangle((21,38+bob,44,50+bob),4,fill='#243047');d.rectangle((25,42+bob,42,47+bob),fill='#354460')
 d.line([(23,39+bob),(28,43+bob),(43,39+bob)],fill=CYAN,width=1);d.rectangle((33,43+bob,35,45+bob),fill=GOLD)
 hy=4 if sleep else (1 if ponder else 0)
 # Oversized head, just a few chunky clusters for highlights.
 d.ellipse((23,13+bob+hy,49,38+bob+hy),fill=INK);d.ellipse((25,15+bob+hy,47,36+bob+hy),fill=GOLD)
 d.ellipse((26,16+bob+hy,42,29+bob+hy),fill=LIGHT);d.rectangle((29,17+bob+hy,34,18+bob+hy),fill='#fff6d2')
 d.polygon([(32,15+bob+hy),(33,10+bob+hy),(36,13+bob+hy),(39,11+bob+hy),(39,16+bob+hy)],fill=INK)
 d.rectangle((34,13+bob+hy,36,16+bob+hy),fill=GOLD)
 # A cyan monocle visor leaves the bill and cheek readable as a duck.
 d.rounded_rectangle((31,21+bob+hy,47,29+bob+hy),2,fill=INK)
 d.rectangle((33,22+bob+hy,45,27+bob+hy),fill='#18364a');d.line((34,22+bob+hy,43,22+bob+hy),fill='#4c879b')
 if sleep:d.line((39,25+bob+hy,44,25+bob+hy),fill=CYAN,width=1)
 else:
  blink=.83<t%1<.9
  d.rectangle((40,24+bob+hy,42,24+bob+hy if blink else 26+bob+hy),fill=CYAN)
  if not blink:d.point((41,24+bob+hy),fill='#d2ffff')
 d.rectangle((28,23+bob+hy,31,27+bob+hy),fill='#626789');d.rectangle((29,24+bob+hy,30,25+bob+hy),fill=MAGENTA)
 d.rounded_rectangle((43,29+bob+hy,56,35+bob+hy),2,fill=INK)
 d.rectangle((45,30+bob+hy,54,32+bob+hy),fill='#f7a856');d.line((47,31+bob+hy,53,31+bob+hy),fill='#ffe0a0')
 d.rectangle((45,33+bob+hy,51,33+bob+hy),fill='#c57a3e')
 # Wing, optionally lifted under the bill to ponder.
 if ponder:
  d.rounded_rectangle((35,34+bob,44,45+bob),3,fill=INK);d.rounded_rectangle((36,35+bob,42,42+bob),2,fill=GOLD)
 else:
  d.ellipse((26,37+bob,35,46+bob),fill=INK);d.ellipse((27,38+bob,34,44+bob),fill=GOLD);d.line((28,39+bob,31,39+bob),fill=LIGHT)
 if sleep:
  d.line([(48,14),(52,14),(48,18),(52,18)],fill=CYAN,width=1)
 return im

def robot(t=0,action='idle'):
 im,d=canvas();bob=round(math.sin(t*math.tau));y=bob
 for x in (22-round(math.sin(t*math.tau)*2) if action=='walk' else 22,37+round(math.sin(t*math.tau)*2) if action=='walk' else 37):
  d.rounded_rectangle((x,46,x+9,54),2,fill=INK);d.rectangle((x+1,49,x+7,52),fill='#4b5c83');d.line((x+2,50,x+6,50),fill='#7893ad')
 d.rounded_rectangle((20,33+y,46,50+y),5,fill=INK);d.rounded_rectangle((22,35+y,44,48+y),4,fill='#3b5272')
 d.rectangle((25,39+y,41,45+y),fill='#1b2d44');d.rectangle((31,40+y,34,43+y),fill=MAGENTA)
 d.rounded_rectangle((15,34+y,23,46+y),3,fill=INK);d.rectangle((17,36+y,20,43+y),fill='#84c7d1')
 d.rounded_rectangle((44,34+y,51,46+y),3,fill=INK);d.rectangle((46,36+y,48,43+y),fill='#84c7d1')
 d.line((33,10+y,33,15+y),fill='#8598bf',width=2);d.rectangle((31,7+y,35,11+y),fill=INK);d.rectangle((32,8+y,34,10+y),fill=MAGENTA)
 d.rounded_rectangle((16,14+y,50,37+y),6,fill=INK);d.rounded_rectangle((18,16+y,48,35+y),5,fill='#78b8c5')
 d.rectangle((22,16+y,41,18+y),fill='#b9efed');d.rectangle((18,21+y,20,30+y),fill='#b9efed')
 d.rounded_rectangle((22,20+y,46,32+y),3,fill=INK);d.rectangle((24,21+y,43,22+y),fill='#183547')
 for x in (27,37):d.rectangle((x,24+y,x+3,24+y if action=='sleep' or .83<t%1<.9 else 27+y),fill=CYAN)
 d.line([(31,30+y),(33,31+y),(35,30+y)],fill='#9adee4',width=1)
 d.rectangle((15,22+y,17,28+y),fill='#505578');d.rectangle((49,22+y,51,28+y),fill='#505578')
 return im

def cat(t=0,action='idle'):
 im,d=canvas();y=round(math.sin(t*math.tau))
 # Curled tail, round torso and stubby paws.
 d.arc((7,32+y,28,51+y),60,310,fill=INK,width=7);d.arc((9,34+y,26,49+y),60,310,fill='#899bc3',width=3)
 d.ellipse((20,29+y,45,52+y),fill=INK);d.ellipse((22,31+y,43,50+y),fill='#7a8cb4')
 d.ellipse((27,36+y,40,49+y),fill='#b8c5e0')
 for x in (23-round(math.sin(t*math.tau)*2) if action=='walk' else 23,36+round(math.sin(t*math.tau)*2) if action=='walk' else 36):d.rounded_rectangle((x,46,x+8,54),2,fill=INK);d.rectangle((x+1,48,x+6,52),fill='#b8c5e0')
 d.polygon([(19,26+y),(18,10+y),(29,15+y),(37,15+y),(48,9+y),(48,27+y)],fill=INK)
 d.polygon([(21,23+y),(21,14+y),(29,18+y),(39,18+y),(45,13+y),(45,25+y)],fill='#93a9cc')
 d.polygon([(22,16+y),(23,22+y),(27,20+y)],fill=MAGENTA);d.polygon([(41,19+y),(44,15+y),(44,22+y)],fill=MAGENTA)
 d.rounded_rectangle((18,19+y,49,38+y),6,fill=INK);d.rounded_rectangle((20,20+y,47,36+y),5,fill='#93a9cc')
 d.rectangle((27,20+y,37,22+y),fill='#c4d3e8')
 d.rounded_rectangle((22,25+y,44,31+y),2,fill='#1c2f48')
 d.rectangle((25,26+y,28,26+y if action=='sleep' or .83<t%1<.9 else 28+y),fill=CYAN);d.rectangle((38,26+y,41,26+y if action=='sleep' or .83<t%1<.9 else 28+y),fill=CYAN)
 d.polygon([(31,30+y),(35,30+y),(33,32+y)],fill=MAGENTA)
 d.line([(30,33+y),(32,34+y),(33,33+y),(34,34+y),(36,33+y)],fill=INK,width=1)
 d.rectangle((23,37+y,42,39+y),fill='#284151');d.rectangle((32,38+y,35,40+y),fill=CYAN)
 return im


def action_frame(kind, action, phase):
    mode=['walk','idle','sleep','idle','idle','idle','ponder','idle'][action]
    im=[duck,robot,cat][kind](phase,mode)
    d=ImageDraw.Draw(im)
    if action==3: # Glance with a small whole-head offset.
        shift=round(math.sin(phase*math.tau))
        head=im.crop((15,5,59,37));im.paste((0,0,0,0),(15,5,59,37));im.paste(head,(15+shift,5))
    if action==4:
        d.rectangle((53,13,54,18),fill=MAGENTA);d.rectangle((53,21,54,22),fill=MAGENTA)
    if action==5: # Wave / stretch one short arm or wing.
        high=round(4*math.sin(phase*math.pi)**2)
        tone=GOLD if kind==0 else '#84c7d1' if kind==1 else '#b8c5e0'
        d.rounded_rectangle((42,28-high,50,41),3,fill=INK)
        d.rounded_rectangle((44,29-high,48,38),2,fill=tone)
    if action==6 and kind!=0: # Paw/hand up under the cheek.
        d.rounded_rectangle((35,31,44,44),3,fill=INK)
        d.rounded_rectangle((37,32,42,40),2,fill='#84c7d1' if kind==1 else '#b8c5e0')
    if action==2:
        d.line([(51,8),(55,8),(51,12),(55,12)],fill=CYAN)
    if action==7:
        hop=round(math.sin(phase*math.pi)**2*5)
        shifted=Image.new('RGBA',im.size);shifted.paste(im,(0,-hop));im=shifted
    return im

if __name__=='__main__':
    import argparse,struct
    parser=argparse.ArgumentParser();parser.add_argument('--out',type=Path,required=True);args=parser.parse_args()
    frames=[action_frame(k,a,f/24) for k in range(3) for a in range(8) for f in range(24)]
    colours={(0,0,0,0)}
    for im in frames:colours.update(im.get_flattened_data())
    colours=sorted(colours);assert colours[0]==(0,0,0,0) and len(colours)<=64
    palette={c:i for i,c in enumerate(colours)};payload=bytearray();offsets=[0]
    for im in frames:
        values=[palette[c] for c in im.get_flattened_data()];i=0
        while i<len(values):
            end=i+1
            while end<len(values) and values[end]==values[i] and end-i<255:end+=1
            payload.extend((end-i,values[i]));i=end
        offsets.append(len(payload))
    out=bytearray(b'RHSPR001')+struct.pack('<HBB',len(frames),len(colours),64)
    for c in colours:out.extend(c)
    for offset in offsets:out.extend(struct.pack('<I',offset))
    out.extend(payload);args.out.parent.mkdir(parents=True,exist_ok=True);args.out.write_bytes(out)
    print(len(frames),'frames;',len(colours),'colours;',len(out),'embedded bytes')
