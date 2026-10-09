#!/usr/bin/env python3
"""Exercise the isolated skin showcase under a private Xvfb display.
Usage: xvfb-run -a python3 scripts/visual/skin_kitty.py BINARY KITTY KITTEN OUTPUT
Never injects input into an existing user's window.
"""
import json, os, pathlib, subprocess, sys, tempfile, time
binary,kitty,kitten,output=sys.argv[1:]
out=pathlib.Path(output).resolve();out.mkdir(parents=True,exist_ok=True)
with tempfile.TemporaryDirectory(prefix='amp-skin-kitty-') as temp:
 root=pathlib.Path(temp);address=f'unix:{root}/rc.sock';trace=root/'trace.jsonl';status=root/'status'
 # Positional shell parameters preserve executable paths without interpolation.
 command=['sh','-c','"$1" --kitty; result=$?; printf "%s" "$result" > "$2"','skin-proof',str(pathlib.Path(binary).resolve()),str(status)]
 log=(out/'kitty.log').open('w')
 proc=subprocess.Popen([kitty,'--config','/dev/null','--hold','--listen-on',address,'-o','allow_remote_control=yes','-o','linux_display_server=x11','-o','remember_window_size=no','-o','initial_window_width=1600','-o','initial_window_height=500','-o','window_padding_width=0',*command],env={**os.environ,'STAR_SKIN_PROOF_TRACE':str(trace)},stdout=log,stderr=log)
 def rc(*args):return subprocess.check_output([kitten,'@','--to',address,*args],stderr=subprocess.PIPE,timeout=10)
 def latest():
  try:return json.loads(trace.read_text().strip().splitlines()[-1])
  except (FileNotFoundError,IndexError,json.JSONDecodeError):return {}
 def wait(predicate):
  deadline=time.monotonic()+15
  while time.monotonic()<deadline:
   value=latest()
   if value and predicate(value):return value
   if proc.poll() is not None:raise RuntimeError('Kitty exited: '+(out/'kitty.log').read_text())
   time.sleep(.05)
  raise AssertionError(f'Timed out; last state: {latest()}')
 def key(k):rc('send-key',k)
 def mouse(button,x,y,up=False):rc('send-text',f'\x1b[<{button};{x};{y}{"m" if up else "M"}')
 try:
  first=wait(lambda s:True);rc('screenshot',str(out/'kitty-initial.png'))
  key('b');wait(lambda s:s['bitmap']);key('b');wait(lambda s:not s['bitmap'])
  key('tab');wait(lambda s:s['focus']==0);key('tab');wait(lambda s:s['focus']==1)
  key('enter');wait(lambda s:not s['playing']);key('space');wait(lambda s:s['playing'])
  # Cell coordinates are derived from the viewport used by the same renderer.
  s=latest();cellx=s['width']/s['columns'];celly=s['height']/s['rows']
  x,y=int(287/cellx)+1,int(201/celly)+1
  mouse(0,x,y);mouse(0,x,y,True);wait(lambda s:s['shuffle'])
  y=int(169/celly)+1;x=int(180/cellx)+1
  mouse(0,x,y);mouse(32,int(s['width']*.8/cellx)+1,y);mouse(0,int(s['width']*.8/cellx)+1,y,True);wait(lambda s:s['position']>60)
  rc('screenshot',str(out/'kitty-interaction.png'))
  key('r');wait(lambda s:s['rigid']);key('d');wait(lambda s:s['density']==2)
  rc('screenshot',str(out/'kitty-density-2x.png'))
  key('d');wait(lambda s:s['density']==1)
  rc('resize-os-window','--unit','pixels','--width','1100','--height','450');wait(lambda s:s['width']<1200)
  rc('screenshot',str(out/'kitty-resized.png'))
  key('u');time.sleep(.25)
  rc('screenshot',str(out/'kitty-unicode.png'))
  key('q');deadline=time.monotonic()+5
  while not status.exists() and time.monotonic()<deadline:time.sleep(.05)
  assert status.read_text()=='0','Showcase did not exit cleanly'
  (out/'trace.jsonl').write_text(trace.read_text())
  states=[json.loads(line) for line in trace.read_text().splitlines()]
  report={'frames':len(states),'initial_render_ms':states[0]['render_ms'],'max_warm_render_ms':max(s['render_ms'] for s in states[1:]),'bytes_total':sum(s['bytes'] for s in states),'checks':['outline/bitmap','keyboard focus','play state','mouse shuffle','drag seek','rigid corners','2x density','resize','Unicode','clean quit']}
  (out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
 finally:
  proc.terminate()
  try:proc.wait(timeout=5)
  except subprocess.TimeoutExpired:proc.kill();proc.wait()
