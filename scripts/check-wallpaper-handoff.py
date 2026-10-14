#!/usr/bin/env python3
"""Check rendered wallpaper handoffs without altering the selected theme or layout."""
import json,os,pathlib,subprocess,time,zlib,struct,threading,argparse
p=argparse.ArgumentParser();p.add_argument('--evidence',type=pathlib.Path,required=True);args=p.parse_args();args.evidence.mkdir(parents=True,exist_ok=True)
def query(name):return json.loads(subprocess.check_output(['niri','msg','--json',name]))
def png(path,rgb):
    w,h=2048,1536
    def chunk(t,b):return struct.pack('!I',len(b))+t+b+struct.pack('!I',zlib.crc32(t+b))
    path.write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('!IIBBBBB',w,h,8,2,0,0,0))+chunk(b'IDAT',zlib.compress((b'\0'+bytes(rgb)*w)*h))+chunk(b'IEND',b''))
colors=[(20,190,70),(150,30,220)];files=[args.evidence/'a.png',args.evidence/'b.png']
for path,color in zip(files,colors):png(path,color)
focused=next(w for w in query('workspaces') if w['is_focused']);empty=next(w for w in query('workspaces') if w['output']==focused['output'] and w['active_window_id'] is None)
output=query('outputs')[focused['output']]['logical'];point=f"{output['x']+400},{output['y']+300} 1x1"
root=pathlib.Path(os.environ.get("XDG_STATE_HOME",str(pathlib.Path.home()/".local/state")))/"dojo/themes"
original=os.readlink(root/"current")
proc=None;frames=[];calibrated=[];capturing=threading.Event()
def pixel():
    image=subprocess.check_output(["grim","-t","ppm","-g",point,"-"],timeout=3)
    return tuple(image.split(b"\n",3)[3][:3])
try:
    subprocess.run(['niri','msg','action','focus-workspace',str(empty['idx'])],check=True,stdout=subprocess.DEVNULL)
    proc=subprocess.Popen(['devenv','shell','--','target/release/examples/wallpaper-handoff-probe',*map(str,files)],stdout=subprocess.PIPE,text=True)
    for line in proc.stdout:
        if line.strip().startswith('CALIBRATE_'):calibrated.append(pixel())
        if line.strip()=='READY':break
    else:raise RuntimeError('probe did not start')
    capturing.set()
    def read_end():
        for line in proc.stdout:
            if line.strip()=='DONE':capturing.clear()
    reader=threading.Thread(target=read_end,daemon=True);reader.start()
    while proc.poll() is None and capturing.is_set():
        rgb=pixel();
        if capturing.is_set():frames.append(rgb)
    assert proc.wait(timeout=5)==0,'probe failed'
finally:
    if proc and proc.poll() is None:proc.terminate();proc.wait(timeout=5)
    restore=root/f".probe-python-restore-{os.getpid()}"
    os.symlink(original,restore);os.replace(restore,root/"current")
    time.sleep(.3)
    subprocess.run(['niri','msg','action','focus-workspace',str(focused['idx'])],check=True,stdout=subprocess.DEVNULL)
    (args.evidence/'frames.json').write_text(json.dumps(frames))
wrong=[rgb for rgb in frames if not any(max(abs(a-b) for a,b in zip(rgb,c))<=2 for c in calibrated)]
print(json.dumps({'calibrated_colors':calibrated,'frames':len(frames),'unexpected_frames':len(wrong),'unexpected_colors':sorted(set(wrong))[:8]}))
assert len(frames)>10 and len(set(calibrated))==2 and all(c in frames for c in calibrated),'capture missed a replacement wallpaper'
assert not wrong,'transparent handoff exposed the underlying background'
