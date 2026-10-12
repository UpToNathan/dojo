#!/usr/bin/env python3
"""Capture repeated real transcript/reply transitions for visual inspection."""
import argparse
import json
import os
import socket
import subprocess
import time
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--evidence', type=Path, required=True)
parser.add_argument('--cycles', type=int, default=3)
parser.add_argument('--long', action='store_true')
args = parser.parse_args()
args.evidence.mkdir(parents=True, exist_ok=True)

def send(event, value):
    with socket.socket(socket.AF_UNIX) as connection:
        connection.connect(str(Path(os.environ['XDG_RUNTIME_DIR']) / 'dojo.sock'))
        connection.sendall((json.dumps({'event': event, 'value': value}) + '\n').encode())
        assert connection.recv(100).strip() == b'queued'

outputs = json.loads(subprocess.check_output(['niri', 'msg', '--json', 'outputs']))
output = next(value for value in outputs.values() if value.get('logical'))
geometry = output['logical']
width, height = geometry['width'], geometry['height']
crop_width = min(width, 700)
crop_height = min(height - 40, 750 if args.long else 260)
crop = f"{geometry['x'] + (width-crop_width)//2},{geometry['y'] + height-crop_height-20} {crop_width}x{crop_height}"
for cycle in range(args.cycles):
    send('phase', 'listening')
    send('transcript', 'Open a browser window and show me the result.')
    time.sleep(.3)
    reply = 'Your browser is open in a new window.'
    if args.long:
        reply = ('Dojo is your Linux desktop shell. It opens applications and helps you operate your computer. ' * 8).strip()
    send('response', reply)
    send('phase', 'settled')
    for frame in range(16):
        subprocess.run(['grim', '-g', crop, str(args.evidence / f'{cycle:02}-{frame:02}.png')], check=True)
    time.sleep(.1)
print(f'Captured {args.cycles} transitions in {args.evidence}; inspect frames for jumping or flicker.')
