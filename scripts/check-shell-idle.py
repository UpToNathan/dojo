#!/usr/bin/env python3
"""Detect sustained redraw work in a quiet resident shell after a UI closes."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time


def snapshot(pid):
    root = Path('/proc') / str(pid)
    stat = (root / 'stat').read_text().rsplit(')', 1)[1].split()
    clients = {}
    for descriptor in (root / 'fdinfo').iterdir():
        fields = dict(line.split(':', 1) for line in descriptor.read_text().splitlines() if ':' in line)
        if 'drm-client-id' in fields:
            key = fields.get('drm-pdev', '').strip() + '/' + fields['drm-client-id'].strip()
            clients[key] = int(fields.get('drm-engine-gfx', '0').split()[0])
    return {'start_ticks': int(stat[19]), 'cpu_ticks': int(stat[11]) + int(stat[12]),
            'gfx': clients, 'time': time.monotonic()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pid', type=int)
    parser.add_argument('--seconds', type=float, default=5)
    parser.add_argument('--max-gfx-ms', type=float, default=1)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not 0 < args.seconds <= 60 or args.max_gfx_ms < 0:
        parser.error('seconds must be within (0,60] and max-gfx-ms nonnegative')
    pid = args.pid or int(subprocess.check_output(['systemctl', '--user', 'show',
        'app-dojo@autostart.service', '-p', 'MainPID', '--value'], text=True))
    before = snapshot(pid)
    time.sleep(args.seconds)
    after = snapshot(pid)
    if before['start_ticks'] != after['start_ticks']:
        raise SystemExit('Process identity changed during the observation.')
    if not before['gfx'] or before['gfx'].keys() != after['gfx'].keys():
        raise SystemExit('GPU client identity missing or changed; result is inconclusive.')
    elapsed = after['time'] - before['time']
    gpu_ms = sum(after['gfx'][k] - before['gfx'][k] for k in before['gfx']) / 1e6
    result = {'pid': pid, 'seconds': elapsed, 'cpu_percent_one_core':
        (after['cpu_ticks'] - before['cpu_ticks']) / os.sysconf('SC_CLK_TCK') / elapsed * 100,
        'gpu_execution_ms': gpu_ms, 'maximum_gpu_ms': args.max_gfx_ms,
        'passed': gpu_ms <= args.max_gfx_ms}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
