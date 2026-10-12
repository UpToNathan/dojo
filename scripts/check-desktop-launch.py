#!/usr/bin/env python3
"""Launch the default browser through the real MCP and verify its Niri window."""
import json
import os
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parents[1]
environment = dict(os.environ)
for name in ['DISPLAY', 'WAYLAND_DISPLAY', 'XDG_DATA_DIRS']:
    environment.pop(name, None)
worker = subprocess.Popen([str(root / 'target/debug/dojo-agent'), '--name', 'launch-check'],
                          stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=environment)
def request(number, method, params):
    worker.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': number, 'method': method, 'params': params}) + '\n')
    worker.stdin.flush()
    while True:
        line = worker.stdout.readline()
        assert line, 'MCP exited'
        value = json.loads(line)
        if value.get('id') == number:
            assert 'error' not in value, value
            return value['result']
try:
    request(1, 'initialize', {'protocolVersion': '2025-11-25', 'capabilities': {}, 'clientInfo': {'name': 'launch-check', 'version': '1'}})
    worker.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
    worker.stdin.flush()
    result = request(2, 'tools/call', {'name': 'desktop_launch', 'arguments': {'application': 'browser'}})
    assert not result.get('isError'), result
    value = json.loads(result['content'][0]['text'])
    assert value['verified'] is True, value
    assert value['window']['app_id'], value
    print('PASS: desktop browser window verified:', value['window']['app_id'], value['window']['id'])
finally:
    worker.stdin.close()
    worker.wait(timeout=10)
