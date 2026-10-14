#!/usr/bin/env python3
"""Retire only the original Astro startup provider; preserve other Niri settings."""
import os
from pathlib import Path
import subprocess
import time

home=Path.home()
config=Path(os.environ.get('XDG_CONFIG_HOME',home/'.config'))/'niri/config.kdl'
source=config.read_text()
old=f'spawn-at-startup "btrbg" "{home}/Pictures/astro.jpg"'
if old not in source:
    print('The original Astro startup provider is already absent.')
    raise SystemExit(0)
if source.count(old)!=1:
    raise SystemExit('Expected one original startup provider; no configuration changed')
root=Path(os.environ.get('XDG_STATE_HOME',home/'.local/state'))/'dojo/wallpaper-owner'
root.mkdir(parents=True,exist_ok=True)
generated=root/'niri.kdl'
generated.write_text(source.replace(old+'\n',''))
pid=subprocess.check_output(['pgrep','-xo','niri'],text=True).strip()
binary=Path(f'/proc/{pid}/exe').resolve()
subprocess.run([str(binary),'validate','--config',str(generated)],check=True)
backup=config.with_name(f'config.kdl.backup-{time.time_ns()}')
config.rename(backup)
config.symlink_to(generated)
subprocess.run([str(binary),'msg','action','load-config-file'],check=True)
print(f'Dojo now owns wallpaper startup; previous config: {backup}')
