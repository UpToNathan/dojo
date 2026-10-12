#!/usr/bin/env python3
"""Verify a browser-opening request through Dojo's actual voice dispatch path."""
import json
import os
import subprocess
import time
from pathlib import Path

root = Path(__file__).resolve().parents[1]
state = Path(os.environ.get('XDG_STATE_HOME', str(Path.home() / '.local/state'))) / 'dojo/voice/conversations.json'
def windows():
    return json.loads(subprocess.check_output(['niri', 'msg', '--json', 'windows']))
status = json.loads(subprocess.check_output([str(root / 'tools/speech/dojo-voice'), 'status']))
assert not status['recording'] and not status['transcribing'] and not status['runId'], 'Voice is busy'
expected = subprocess.check_output(['xdg-settings', 'get', 'default-web-browser'], text=True).strip().removesuffix('.desktop')
assert expected, 'Default browser is missing'
before = {window['id'] for window in windows()}
reply = json.loads(subprocess.check_output([str(root / 'tools/speech/dojo-voice'), 'send', 'Open a browser window.']))
deadline = time.monotonic() + 45
while time.monotonic() < deadline:
    chat = next(c for c in json.loads(state.read_text())['chats'] if c['threadId'] == reply['threadId'])
    new = [w for w in windows() if w['id'] not in before and w.get('app_id', '').lower() == expected.lower()]
    if chat.get('settledAt'):
        assert chat['status'] == 'completed', chat['status']
        assert new, 'Agent completed without a desktop window'
        options = chat['modelSelection'].get('options', [])
        assert {'id': 'reasoningEffort', 'value': 'none'} in options, options
        print('PASS: voice request completed with a new desktop window and explicit zero reasoning')
        print('Thread:', reply['threadId'])
        break
    time.sleep(0.25)
else:
    raise AssertionError('Voice browser request did not complete within 45 seconds')
