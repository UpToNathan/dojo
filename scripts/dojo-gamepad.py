#!/usr/bin/env python3
"""USB DualSense controls for Dojo's launcher and Niri focus."""
import array
import fcntl
import glob
import json
import os
from pathlib import Path
import queue
import select
import shlex
import signal
import socket
import struct
import subprocess
import threading
import time

JS_EVENT = struct.Struct("IhBB")
BTN_CROSS, BTN_CIRCLE, BTN_SQUARE, BTN_TRIANGLE = 304, 305, 308, 307
ABS_X, ABS_Y, ABS_RX, ABS_RY, ABS_HAT0X, ABS_HAT0Y = 0, 1, 3, 4, 16, 17
APPS = {
    't3': (('com.t3tools.T3Code.desktop',), ('com.t3tools.T3Code', 't3code')),
}


def environment_command(variable):
    command = shlex.split(os.environ.get(variable, ''))
    if not command:
        raise RuntimeError(f'${variable} is not set in the controller service environment')
    return command


def open_app(name):
    if name in ('browser', 'editor', 'terminal'):
        if name == 'editor':
            command = [*environment_command('TERMINAL'), '-e', *environment_command('EDITOR')]
        else:
            command = environment_command(name.upper())
        subprocess.run(['uwsm', 'app', '-t', 'service', '--', *command], check=True)
        return
    command, app_ids = APPS[name]
    windows = json.loads(subprocess.check_output(['niri', 'msg', '--json', 'windows'], text=True))
    existing = [window for window in windows if window.get('app_id') in app_ids]
    if existing:
        window = max(existing, key=lambda w: (w.get('focus_timestamp') or {}).get('secs', 0))
        subprocess.run(['niri', 'msg', 'action', 'focus-window', '--id', str(window['id'])], check=True)
    else:
        subprocess.run(['uwsm', 'app', '-t', 'service', '--', *command], check=True)


def cycle_focus():
    windows = json.loads(subprocess.check_output(['niri', 'msg', '--json', 'windows'], text=True))
    if windows:
        subprocess.run(['niri', 'msg', 'action', 'focus-window-previous'], check=True)
        return
    outputs = json.loads(subprocess.check_output(['niri', 'msg', '--json', 'outputs'], text=True))
    if len(outputs) > 1:
        subprocess.run(['niri', 'msg', 'action', 'focus-monitor-next'], check=True)


def devices():
    for name in sorted(glob.glob('/sys/class/input/js*')):
        device = Path(name) / 'device'
        try:
            ids = [(device / 'id' / field).read_text().strip() for field in ('bustype', 'vendor', 'product')]
            if ids[0] == '0003' and ids[1] == '054c' and ids[2] in ('0ce6', '0df2'):
                if 'Motion Sensors' not in (device / 'name').read_text():
                    yield '/dev/input/' + Path(name).name
        except OSError:
            continue


def request(command):
    runtime = Path(os.environ['XDG_RUNTIME_DIR'])
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(3)
        client.connect(str(runtime / 'dojo.sock'))
        client.sendall((command + '\n').encode())
        response = bytearray()
        while b'\n' not in response:
            part = client.recv(4096)
            if not part:
                raise RuntimeError('Dojo closed the control connection')
            response.extend(part)
        value = response.decode().strip()
        if value != 'queued':
            raise RuntimeError(value)
        return value


class Controls:
    def __init__(self, emit):
        self.emit = emit
        self.axes = {}
        self.directions = {}

    def button(self, code, pressed, initial=False):
        if initial:
            return
        if not pressed:
            return
        command = {BTN_SQUARE: 'launcher', BTN_CROSS: 'launcher-activate',
                   BTN_CIRCLE: 'launcher-dismiss', BTN_TRIANGLE: 'cycle-focus'}.get(code)
        if command:
            self.emit('dojo' if command != 'cycle-focus' else 'niri', command)

    def axis(self, code, value, initial=False):
        if code in (ABS_HAT0X, ABS_HAT0Y):
            previous = self.axes.get(code, 0)
            self.axes[code] = 0 if initial else value
            if not initial and value and value != previous:
                app = ('browser' if value < 0 else 'terminal') if code == ABS_HAT0X else ('editor' if value < 0 else 't3')
                self.emit('app', app)
        elif code in (ABS_X, ABS_Y, ABS_RX, ABS_RY):
            # Startup events describe existing state, not a user's new action.
            self.axes[code] = 0 if initial else value

    def tick(self, now):
        for axis, negative, positive in (
            (ABS_X, 'focus-column-left', 'focus-column-right'),
            (ABS_Y, 'focus-window-up', 'focus-window-down'),
            (ABS_RY, 'launcher-up', 'launcher-down'),
        ):
            value = self.axes.get(axis, 0)
            direction = (1 if value > 16000 else -1 if value < -16000 else 0)
            previous, next_repeat = self.directions.get(axis, (0, 0))
            if direction != previous:
                self.directions[axis] = (direction, now + .4)
                if direction:
                    target = 'dojo' if axis == ABS_RY else 'niri'
                    self.emit(target, positive if direction > 0 else negative)
            elif direction and now >= next_repeat:
                target = 'dojo' if axis == ABS_RY else 'niri'
                self.emit(target, positive if direction > 0 else negative)
                self.directions[axis] = (direction, now + .12)

    def reset(self):
        self.axes.clear()
        self.directions.clear()


def maps(fd):
    axes = array.array('B', [0] * 64)
    buttons = array.array('H', [0] * 512)
    fcntl.ioctl(fd, 0x80406a32, axes, True)
    fcntl.ioctl(fd, 0x84006a34, buttons, True)
    return axes, buttons


def run():
    commands = queue.Queue()
    stopped = threading.Event()

    def emit(target, command):
        commands.put((target, command))

    def worker():
        while True:
            item = commands.get()
            try:
                if item is None:
                    return
                target, command = item
                if target == 'app':
                    open_app(command)
                    continue
                if target == 'niri':
                    if command == 'cycle-focus':
                        cycle_focus()
                    else:
                        subprocess.run(['niri', 'msg', 'action', command], check=True)
                    continue
                request(command)
            except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
                print(f'Dojo controller: {error}', flush=True)
            finally:
                commands.task_done()

    thread = threading.Thread(target=worker, daemon=True)
    thread.start()
    controls = Controls(emit)
    for sig in (signal.SIGTERM, signal.SIGINT):
        signal.signal(sig, lambda *_: stopped.set())
    while not stopped.is_set():
        paths = list(devices())
        if not paths:
            stopped.wait(1)
            continue
        try:
            with open(paths[0], 'rb', buffering=0) as device:
                axes, buttons = maps(device.fileno())
                print(f'Dojo controller connected: {paths[0]}', flush=True)
                while not stopped.is_set():
                    ready, _, _ = select.select([device], [], [], .02)
                    if ready:
                        data = device.read(JS_EVENT.size)
                        if len(data) != JS_EVENT.size:
                            raise OSError('Controller disconnected')
                        _, value, kind, number = JS_EVENT.unpack(data)
                        initial = bool(kind & 0x80)
                        if kind & 1 and number < len(buttons):
                            controls.button(buttons[number], bool(value), initial)
                        elif kind & 2 and number < len(axes):
                            controls.axis(axes[number], value, initial)
                    controls.tick(time.monotonic())
        except OSError as error:
            print(f'Dojo controller: {error}', flush=True)
        finally:
            controls.reset()
        stopped.wait(1)
    controls.reset()
    commands.put(None)
    thread.join(timeout=5)


if __name__ == '__main__':
    run()
