#!/usr/bin/env python3
"""Generate public, editable Excalidraw fixtures for the native app's own renderer."""
import json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
out=ROOT/'design/product/fixtures';out.mkdir(parents=True,exist_ok=True)
class Scene:
 def __init__(self):self.items=[]
 def shape(self,kind,x,y,w,h,fill='transparent',stroke='#4265a0',**extra):
  ident=f'item{len(self.items):04d}'
  self.items.append(dict(id=ident,type=kind,x=x,y=y,width=w,height=h,angle=0,strokeColor=stroke,backgroundColor=fill,fillStyle='solid',strokeWidth=1.5,roughness=0,opacity=100,seed=len(self.items)+17,version=1,versionNonce=1,isDeleted=False,groupIds=[]));self.items[-1].update(extra);return ident
 def text(self,s,x,y,w=240,size=20,color='#202020',**kw):return self.shape('text',x,y,w,size*1.5,stroke=color,text=s,originalText=s,fontSize=size,fontFamily=2,textAlign='left',verticalAlign='top',lineHeight=1.25,**kw)
 def box(self,title,detail,x,y,w=240,fill='#f0f3f8'):
  self.shape('rectangle',x,y,w,100,fill,roundness={'type':3,'value':8});self.text(title,x+18,y+18,w-36,20);self.text(detail,x+18,y+54,w-36,13,'#626262')
 def arrow(self,x,y,dx,dy):self.shape('arrow',x,y,abs(dx) or 1,abs(dy) or 1,points=[[0,0],[dx,dy]],endArrowhead='arrow')
 def sticky(self,text,x,y,color):
  ident=self.shape('stickynote',x,y,190,150,color,'#74644f');self.text(text,x+15,y+18,160,18,'#30271f',containerId=ident)
 def save(self,name):
  (out/f'{name}.excalidraw').write_text(json.dumps({'type':'excalidraw','version':2,'source':'OpenPaperDesign public product fixture','elements':self.items,'appState':{'viewBackgroundColor':'#fafafa'},'files':{}},indent=2)+'\n')
s=Scene();s.text('A place to think. A system you can explain.',0,0,1000,36);s.text('OpenPaperDesign / local desktop + optional AI agents',0,62,900,17,'#626262')
s.box('You','Sketch / review / refine',0,165);s.box('OpenPaperDesign','An editable native canvas',360,165,280,'#e0e8f5');s.box('Your project','Pages / JSON / assets',760,165)
s.arrow(245,215,105,0);s.arrow(645,215,105,0);s.box('Codex','Optional MCP client',100,390);s.box('Claude Code','Optional MCP client',680,390);s.arrow(225,385,160,-110);s.arrow(800,385,-180,-110)
s.sticky('Design the interface\nbeside the system.',385,365,'#ffdf6b');s.text('No account. No subscription. Your files.',280,600,800,22)
s.save('architecture')
s=Scene();s.text('A little room for the unfinished idea.',0,0,1100,36);s.text('Notes, sketches and decisions. All still editable.',0,65,1000,17,'#626262')
for i,(label,color) in enumerate([('What are we\ntrying to change?','#ffdf6b'),('Keep the useful\nparts we have.','#ffb8a8'),('Draw it before\nwe build it.','#b5e6a2'),('What data\ndo we need?','#a8d8ff'),('Review it\ntogether.','#d7c2ff'),('Next step:\ntry a small slice.','#e8e8ec')]):s.sticky(label,(i%3)*280,(i//3)*220+145,color)
s.shape('ellipse',900,170,200,120,'transparent',roughness=1.4);s.text('rough idea',927,210,160,18);s.shape('rectangle',885,415,230,115,'transparent',roughness=1.2);s.text('a clearer direction',905,455,220,18);s.arrow(1000,300,0,100)
s.save('whiteboard')
s=Scene();s.text('Three brands. One thread.',0,0,1200,36);s.text('Public interface studies / gustavonline · Arc’IT AI · onlinesourdough',0,65,1200,17,'#626262')
for i,(brand,heading,body,bg,accent) in enumerate([('gustavonline','Work. Ideas.\nExperiments.','Data, software and agentic engineering.','#fafafa','#d37635'),('Arc’IT AI','AI should understand\nyour business.','Your data and business context.','#f3f5f8','#4265a0'),('onlinesourdough','Valuable AI starts\nwith your data.','And your business context.','#f8f1e5','#9c6849')]):
 x=i*450;s.shape('rectangle',x,145,410,510,bg,'#ccc8c0');s.text(brand,x+24,168,360,20);s.shape('line',x+20,211,370,1,points=[[0,0],[370,0]],stroke='#d9d4cb');s.text(heading,x+24,245,360,29);s.text(body,x+24,340,360,13,'#626262')
 for row in range(7):
  for col in range(12):
   levels=['#e9dfcf','#e5ba92',accent,'#d7b99f'] if i!=1 else ['#d8e2f2','#b1c2e0','#7697c6','#4265a0'];s.shape('rectangle',x+25+col*29,420+row*24,24,18,levels[(row*7+col*3)%4],'transparent')
s.text('Layout studies, not browser screenshots.',0,710,1200,16,'#626262');s.save('design')
print(out)
