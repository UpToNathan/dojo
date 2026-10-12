#!/usr/bin/env python3
"""Verify the compositor's voice handoff IPC and idempotent response."""
import argparse
import json
import os
from pathlib import Path
from run import Peer

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--log', required=True)
args = parser.parse_args()
peer = Peer(os.environ['NIRI_SOCKET'])
try:
    focus = peer.request('FocusedWindow')
    first = peer.request('YieldHumanInput')['InputCoordination']
    second = peer.request('YieldHumanInput')['InputCoordination']
    assert not first['human_owned_windows'], first
    assert first == second, (first, second)
    assert peer.request('FocusedWindow') == focus
    Path(args.log).write_text(json.dumps(first) + '\n')
    print('Voice handoff IPC clears ownership, preserves focus and is idempotent.')
finally:
    peer.close()
