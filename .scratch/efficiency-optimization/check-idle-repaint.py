#!/usr/bin/env python3
"""Check GPU idleness after the caller establishes Settled phase and no UI input."""
import argparse
import json
import os
from pathlib import Path
import time


def snapshot(pid):
    process = Path('/proc') / str(pid)
    fields = (process / 'stat').read_text().rsplit(')', 1)[1].split()
    clients = {}
    for descriptor in (process / 'fdinfo').iterdir():
        info = dict(line.split(':', 1) for line in descriptor.read_text().splitlines() if ':' in line)
        if 'drm-client-id' in info:
            key = info.get('drm-pdev', '').strip() + '/' + info['drm-client-id'].strip()
            clients[key] = int(info.get('drm-engine-gfx', '0').split()[0])
    return {'time': time.monotonic(), 'start': int(fields[19]), 'ticks': int(fields[11]) + int(fields[12]), 'clients': clients}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('pid', type=int)
    parser.add_argument('--seconds', type=float, default=3)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    before = snapshot(args.pid)
    time.sleep(args.seconds)
    after = snapshot(args.pid)
    assert before['start'] == after['start'], 'Process identity changed during measurement'
    elapsed = after['time'] - before['time']
    gpu = sum(value - before['clients'][client] for client, value in after['clients'].items() if client in before['clients'])
    result = {'pid': args.pid, 'seconds': elapsed, 'cpu_pct_one_core': (after['ticks'] - before['ticks']) / os.sysconf('SC_CLK_TCK') / elapsed * 100, 'gpu_engine_ns': gpu, 'before': before, 'after': after}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({key: value for key, value in result.items() if key not in ('before', 'after')}))
    assert gpu == 0, 'Quiet shell continued submitting GPU work'


if __name__ == '__main__':
    main()
