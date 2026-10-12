#!/usr/bin/env python3
"""Exercise native capture/model reuse without sending recordings to T3."""
import json
import subprocess
import tempfile
import time
from pathlib import Path

root = Path(__file__).resolve().parents[1]
with tempfile.TemporaryFile(mode='w+') as diagnostics:
    worker = subprocess.Popen([str(root / 'target/debug/dojo-speech'), 'serve'],
                              stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              stderr=diagnostics, text=True)
    def command(action, recording):
        worker.stdin.write(json.dumps({'action': action, 'id': recording}) + '\n')
        worker.stdin.flush()
    def event(expected, recording):
        value = json.loads(worker.stdout.readline())
        assert value['event'] == expected, value
        assert value['id'] == recording, value
        return value
    try:
        ready = json.loads(worker.stdout.readline())
        assert ready == {'event': 'ready'}, ready
        for recording in ['first', 'second']:
            command('begin', recording)
            event('listening', recording)
            time.sleep(1)
            command('finish', recording)
            event('transcribing', recording)
            value = event('transcript', recording)
            assert isinstance(value['text'], str), value
        command('begin', 'cancelled')
        event('listening', 'cancelled')
        command('cancel', 'cancelled')
        command('begin', 'after-cancel')
        event('listening', 'after-cancel')
        command('cancel', 'after-cancel')
        worker.stdin.close()
        assert worker.wait(timeout=10) == 0
        diagnostics.seek(0)
        log = diagnostics.read()
        assert log.count('model ready in ') == 1, log[-2000:]
        print('PASS: two transcripts, one model load, cancellation permits another capture')
    finally:
        if worker.poll() is None:
            worker.kill()
            worker.wait()
